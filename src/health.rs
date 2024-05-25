use crate::{
    config::HealthCheckConfig,
    event::{self, EventType},
    process,
};
use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    net::{TcpStream, ToSocketAddrs},
    process::Command,
    str::FromStr,
    sync::RwLock,
    thread,
    time::Duration,
};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum HealthCheckType {
    Http,
    Tcp,
    Cmd,
    Proccess,
}

impl FromStr for HealthCheckType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "http" => Ok(HealthCheckType::Http),
            "tcp" => Ok(HealthCheckType::Tcp),
            "cmd" => Ok(HealthCheckType::Cmd),
            _ => Ok(HealthCheckType::Proccess),
        }
    }
}

lazy_static! {
    static ref SERVICES_HEALTH_STATUS: RwLock<HashMap<String, i32>> = RwLock::new(HashMap::new());
}

pub fn start_watch(service_name: String, config: Option<HealthCheckConfig>) {
    let health_cfg = config.clone(); // to avoid borrow count
    if health_cfg.is_none() {
        info!("[{}] is not enabled to health check", &service_name);
        return;
    }
    if is_watching(&service_name) {
        return;
    }
    set_watch_flag(&service_name);
    thread::spawn(move || do_watch_health(service_name, config.unwrap()));
    return;
}

pub fn stop_watch(service_name: String) {
    let mut status = SERVICES_HEALTH_STATUS.write().unwrap();
    if !status.contains_key(&service_name) {
        warn!("[{}] is not being watched, ignore stop", &service_name);
        return;
    }
    status.remove(&service_name);
}

fn is_watching(service_name: &str) -> bool {
    let status = SERVICES_HEALTH_STATUS.read().unwrap();
    status.contains_key(service_name)
}

fn set_watch_flag(service_name: &str) {
    let mut status = SERVICES_HEALTH_STATUS.write().unwrap();
    status.insert(service_name.to_owned(), 0);
}

fn do_watch_health(service_name: String, config: HealthCheckConfig) {
    if config.start_period.is_some() {
        thread::sleep(Duration::from_secs(config.start_period.unwrap() as u64));
    }
    if !is_watching(&service_name) {
        return;
    }
    info!("[{}] has enabled health checks", &service_name);
    loop {
        if !is_watching(&service_name) {
            info!(
                "[{}] is not being watched, stop health check",
                &service_name
            );
            break;
        }
        let r = check(&service_name, &config);
        let mut check_interval = config.interval;
        if r.is_err() {
            warn!(
                "[{}] health check has error: {}",
                service_name,
                r.err().unwrap()
            );
            thread::sleep(Duration::from_secs(check_interval as u64));
            continue;
        }
        let success = r.unwrap();
        if !success {
            event::send_process_event(&service_name, EventType::Unhealthy, None, None);
            let fail_times = incr_fail_times(&service_name);
            let restart: bool = fail_times > config.max_failures;
            if restart {
                warn!("health check failure count for [{}] has exceeded the threshold, preparing to restart it", &service_name);
                process::manager::restart_service(&service_name).unwrap_or_else(|err| {
                    warn!("restart [{}] failed: {}", &service_name, err);
                });
                check_interval += config.start_period.unwrap_or(0);
            }
        } else {
            event::send_process_event(&service_name, EventType::Healthy, None, None);
        }
        thread::sleep(Duration::from_secs(check_interval as u64));
    }
}

fn check(service_name: &str, config: &HealthCheckConfig) -> Result<bool> {
    match config.test_type {
        HealthCheckType::Http => return test_with_http(&config.test_target.clone()),
        HealthCheckType::Tcp => return test_with_tcp(&config.test_target.clone()),
        HealthCheckType::Cmd => return test_with_cmd(&config.test_target.clone()),
        _ => return test_with_process(service_name),
    }
}

fn incr_fail_times(service_name: &str) -> i32 {
    let mut service_statuses = SERVICES_HEALTH_STATUS.write().unwrap();
    let fail_times = service_statuses.entry(service_name.to_owned()).or_insert(0);
    *fail_times += 1;
    *fail_times
}

fn test_with_process(service_name: &str) -> Result<bool> {
    Ok(process::status::is_running_by_name(service_name))
}

fn test_with_http(url: &str) -> Result<bool> {
    let req = reqwest::blocking::get(url);
    if req.is_err() {
        return Err(req.unwrap_err().into());
    }
    let status = req.unwrap().status();
    Ok(status.is_success())
}

fn test_with_tcp(address: &str) -> Result<bool> {
    let socket_addrs = address.to_socket_addrs()?.next().ok_or(io::Error::new(
        io::ErrorKind::Other,
        format!("{} {}", "can not convert to address:", address),
    ))?;
    TcpStream::connect_timeout(&socket_addrs, Duration::from_secs(5))
        .map(|_| true)
        .or_else(|_| Ok(false))
}

fn test_with_cmd(cmd: &str) -> Result<bool> {
    // 分割命令字符串以获取命令名和参数
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Err(anyhow!("Command cannot be empty"));
    }
    // 分别获取命令名和参数
    let command = parts[0];
    let args = &parts[1..];
    // 创建并执行命令
    let output = Command::new(command).args(args).output()?;
    // 根据命令的退出状态判断健康状态
    // 这里假设如果命令成功执行（退出状态码为0），则进程健康
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_success() {
        let url = "https://cn.bing.com"; // 替换为一个始终可用的URL
        assert_eq!(test_with_http(url).unwrap(), true);
    }

    #[test]
    fn test_http_failure() {
        let url = "http://thisurldoesnotexist.tld"; // 一个不存在的URL
        assert!(test_with_http(url).is_err());
    }

    #[test]
    fn test_tcp_success() {
        let address = "baidu.com:80"; // 替换为一个始终可用的地址
        assert_eq!(test_with_tcp(address).unwrap(), true);
    }

    #[test]
    fn test_tcp_failure() {
        let address = "256.256.256.256:80"; // 一个无效的地址
        assert_eq!(test_with_tcp(address).is_err(), true);
    }

    #[test]
    fn test_cmd_success() {
        let cmd = "echo Hello World"; // 替换为一个始终成功的命令
        assert_eq!(test_with_cmd(cmd).unwrap(), true);
    }

    #[test]
    fn test_cmd_failure() {
        let cmd = "false"; // 大多数系统上一个始终失败的命令
        assert_eq!(test_with_cmd(cmd).is_err(), true);
    }

    #[test]
    fn test_cmd_empty() {
        let cmd = ""; // 一个空命令
        assert!(test_with_cmd(cmd).is_err());
    }
}
