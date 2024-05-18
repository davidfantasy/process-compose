#[cfg(target_os = "linux")]
use super::platform::linux::{before_exec, kill_process, terminate_process};

#[cfg(target_os = "windows")]
use super::platform::windows::{before_exec, kill_process, terminate_process};
use super::{pending, status};
use crate::config::ServiceConfig;
use crate::event::EventType;
use crate::{env, event};
use anyhow::{Error, Result};
use log::{debug, error, info, warn};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub fn start_services(services: Vec<String>) -> Result<()> {
    if services.len() == 0 {
        return Ok(());
    }
    for name in services.iter() {
        let service_info = status::find_readonly_proc_runtime(name);
        if service_info.is_err() {
            warn!("starting service [{}] not found:", name);
            continue;
        }
        let dep_ok = status::check_dep_ok(name);
        //仅启动没有依赖的服务，其它服务加入待启动列表
        if dep_ok {
            start_service(name)?;
        } else {
            info!("service [{}] has dependencies, add to pending list", name);
            let deps = service_info.unwrap().config.depends_on.clone().unwrap();
            pending::add_pending_service(name, deps)
        }
    }
    Ok(())
}

pub fn start_service(service_name: &str) -> Result<()> {
    let proc_runtime = status::find_readonly_proc_runtime(service_name)?;
    let conf = proc_runtime.config;
    let pid = proc_runtime.pid;
    let svc_name = service_name.to_string();
    if pid.is_some() {
        let pid_val = pid.unwrap();
        if status::is_running_by_pid(pid_val) {
            info!(
                "service [{}] is already running with pid {}!",
                service_name, pid_val
            );
            event::send_process_event(service_name, EventType::Running, None, pid);
            return Ok(());
        }
    }
    thread::spawn(move || {
        if let Err(err) = spawn_proc(Arc::clone(&conf)) {
            error!("service [{}] exited with error: {}", svc_name, err);
        }
    });
    Ok(())
}

pub fn stop_services(services: Vec<String>) -> Result<()> {
    if services.len() == 0 {
        return Ok(());
    }
    for name in services.iter() {
        stop_service(name)?;
    }
    Ok(())
}

pub fn stop_service(service_name: &str) -> Result<()> {
    let proc_runtime = status::find_readonly_proc_runtime(service_name)?;
    let pid = proc_runtime.pid;
    if pid.is_none() {
        info!("service [{}]  is not running!", service_name);
        return Ok(());
    }
    let pid_val = pid.unwrap();
    let mut is_running = status::is_running_by_pid(pid_val);
    //更新进程的主动停止标志位
    status::update_proc_runtime(service_name, |p| {
        p.stopped_by_supervisor = true;
    })?;
    if !is_running {
        info!(
            "ignore stop command, service [{}]  is not running, pid: {}!",
            service_name, pid_val
        );
        return Ok(());
    }
    info!("service [{}] (pid: {}) is stopping", service_name, pid_val);
    //首先尝试通过信号量的方式让进程自己退出
    if let Err(err) = terminate_process(pid_val) {
        warn!("signal {} (pid: {}) failed: {}", service_name, pid_val, err);
    }
    let start_time = Instant::now();
    let timeout_duration = Duration::from_secs(2);
    while is_running && start_time.elapsed() <= timeout_duration {
        thread::sleep(Duration::from_millis(200));
        is_running = status::is_running_by_pid(pid_val);
    }
    //如果超过规定时间进程没有退出，则强制杀掉进程
    if is_running {
        info!("service [{}] (pid: {}) is still running within the specified time after sending the interrupt signal, and is ready to be killed", service_name, pid_val);
        kill_process(pid_val)?;
    }
    Ok(())
}

pub fn restart_service(service_name: &str) -> Result<()> {
    if status::is_running_by_name(service_name) {
        stop_service(service_name)?;
    }
    start_service(service_name)?;
    Ok(())
}

fn spawn_proc(conf: Arc<ServiceConfig>) -> Result<()> {
    let command_args = &conf.start_cmd;
    let (command, params) = command_args.split_first().unwrap();
    let svc_name = &(conf.name);
    let mut current_dir = env::ROOT_DIR.clone();
    current_dir.push(&conf.name);
    let real_cmd = if command.starts_with(".") {
        let mut abs_command = current_dir.clone();
        //同时去掉command路径中的"./"部分，不然push会导致错误
        abs_command.push(&command[2..]);
        abs_command
    } else {
        PathBuf::from(command)
    };
    let mut cmd = Command::new(real_cmd.clone());
    cmd.args(params);
    //设置子进程的工作目录，这会影响子进程中对相对路径的处理,但对于全局命令来说设置可能会导致错误
    if !real_cmd.is_relative() {
        cmd.current_dir(current_dir);
    }
    if conf.log_redirect {
        let log_file = env::create_service_redirect_log_file(svc_name, "out").unwrap();
        let log_file_err = log_file.try_clone()?;
        cmd.stdout(Stdio::from(log_file));
        cmd.stderr(Stdio::from(log_file_err));
    } else {
        let log_file_err = env::create_service_redirect_log_file(svc_name, "err").unwrap();
        cmd.stdout(Stdio::null());
        cmd.stderr(log_file_err);
    }
    if command.starts_with(".") {}
    before_exec(&mut cmd)?;
    debug!("execute service [{}] start command:{}", svc_name, command);
    let child = cmd.spawn().map_err(|e| format!("{}", e));
    match child {
        Ok(mut child_proc) => {
            //更新进程状态为已启动
            status::update_proc_to_started(svc_name, child_proc.id(), true)?;
            let exit_status = child_proc.wait().map_err(|e| format!("{}", e));
            match exit_status {
                Ok(status) => {
                    //进程正常退出
                    status::update_proc_to_stopped(
                        svc_name,
                        format!("exit code: {}", status.code().or(Some(0)).unwrap()).as_str(),
                        child_proc.id(),
                    )?;
                }
                Err(err) => {
                    //进程异常退出
                    status::update_proc_to_stopped(svc_name, err.as_str(), child_proc.id())?;
                }
            }
        }
        Err(err) => {
            return Err(Error::msg(format!("spawn process error: {}", err)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use log::LevelFilter;
    use log4rs::{
        append::console::{ConsoleAppender, Target},
        config::{Appender, Root},
        Config,
    };

    use crate::config::{self, GlobalConfig};

    use super::*;
    use std::{collections::HashMap, sync::mpsc};

    fn init_test_log() {
        let console = ConsoleAppender::builder().target(Target::Stdout).build();
        let config = Config::builder()
            .appender(Appender::builder().build("console", Box::new(console)))
            .build(Root::builder().appender("console").build(LevelFilter::Info))
            .unwrap();
        let _ = log4rs::init_config(config).expect("log init failed!!");
    }

    fn mock_config() -> GlobalConfig {
        let mut services_map = HashMap::new();
        let service_config = ServiceConfig {
            name: "service1".to_string(),
            log_redirect: false,
            log_pattern: None,
            healthcheck: None,
            start_cmd: "timeout /t 10"
                .split_whitespace()
                .map(|s| s.to_string())
                .collect(),
            depends_on: None,
        };
        services_map.insert("service1".to_string(), service_config);
        let global_config = GlobalConfig {
            log_level: "info".to_string(),
            app_data_home: "/app/data".to_string(),
            services: services_map,
            api: None,
            sys_service_name: "process-manager".to_owned(),
            sys_service_desc: "".to_owned(),
        };
        config::set_config(global_config.clone());
        global_config
    }

    #[test]
    fn test_start_service() {
        init_test_log();
        let (sender, receiver) = mpsc::channel();
        event::set_sender(sender);
        let config = mock_config();
        let orders = config.services.keys().cloned().collect();
        let service_name = "service1";
        status::init_processes(&config, orders).unwrap();
        assert!(start_service(service_name).is_ok());
        // Check if the event was sent
        let event = receiver.recv().unwrap();
        assert_eq!(event.service_name, service_name);
        assert_eq!(event.pid.is_some(), true);
        assert_eq!(event.event_type, EventType::Running);
        thread::sleep(Duration::from_millis(100));
        let service_info = status::find_readonly_proc_runtime(service_name).unwrap();
        assert_eq!(service_info.pid.is_some(), true);
        assert_eq!(service_info.is_child_process, true);
        assert_eq!(service_info.last_start_time.is_some(), true);
    }

    #[test]
    fn test_stop_service() {
        init_test_log();
        let (sender, receiver) = mpsc::channel();
        event::set_sender(sender);
        let config = mock_config();
        let orders = config.services.keys().cloned().collect();
        let service_name = "service1";
        status::init_processes(&config, orders).unwrap();
        start_service(service_name).unwrap();
        let _ = receiver.recv().unwrap();
        stop_service(service_name).unwrap();
        let stop_event = receiver.recv().unwrap();
        assert_eq!(stop_event.service_name, service_name);
        assert_eq!(stop_event.pid.is_some(), true);
        assert_eq!(stop_event.event_type, EventType::Stopped);
        thread::sleep(Duration::from_millis(100));
        let service_info = status::find_readonly_proc_runtime(service_name).unwrap();
        assert_eq!(service_info.pid.is_none(), true);
        assert_eq!(service_info.last_stop_time.is_some(), true);
        assert_eq!(service_info.stopped_by_supervisor, true);
    }
}
