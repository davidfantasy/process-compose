#[cfg(target_os = "linux")]
use super::platform::linux::{before_exec, kill_process, terminate_process};

#[cfg(target_os = "windows")]
use super::platform::windows::{before_exec, kill_process, terminate_process};
use crate::config::{GlobalConfig, ServiceConfig};
use crate::env;
use anyhow::{Error, Result};
use log::{error, info, warn};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};
use std::{fs, thread};
use sysinfo::{Pid, System};

#[derive(Clone, Debug)]
struct ProcessRuntimeInfo {
    name: String,
    pid: Option<u32>,
    is_child_process: bool,
    config: Arc<ServiceConfig>,
    stopped_by_supervisor: bool,
    last_start_time: Option<SystemTime>,
    last_stop_time: Option<SystemTime>,
    exit_err: Option<String>,
}

pub struct ProcessEvent {
    pub service_name: String,
    pub pid: Option<u32>,
    pub event_type: EventType,
    pub data: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventType {
    //进程运行成功
    Running = 1,
    //进程被compose主动停止
    Stopped = 2,
    //进程自身退出
    Exited = 3,
}

static PROCESSES: RwLock<Vec<RwLock<ProcessRuntimeInfo>>> = RwLock::new(Vec::new());

pub fn init_processes(config: &GlobalConfig, start_orders: Vec<String>) -> Result<()> {
    let mut process_map: HashMap<String, ServiceConfig> = HashMap::new();
    for (name, svc_config) in config.services.iter() {
        process_map.insert(name.clone(), svc_config.clone());
    }
    let mut processes = PROCESSES.write().unwrap();
    // 按照启动顺序进行排序
    for (_, name) in start_orders.iter().enumerate() {
        if let Some(cfg) = process_map.get(name) {
            let config = Arc::new(cfg.clone());
            let mut proc = find_proc_from_pid_file(config.clone());
            if proc.is_none() {
                proc = Some(ProcessRuntimeInfo {
                    name: name.clone(),
                    pid: None,
                    config: config.clone(),
                    is_child_process: true,
                    stopped_by_supervisor: false,
                    last_start_time: None,
                    last_stop_time: None,
                    exit_err: None,
                });
            }
            processes.push(RwLock::new(proc.unwrap()));
        } else {
            return Err(Error::msg(format!(
                "service {} was not found in the configuration.",
                name
            )));
        }
    }
    Ok(())
}

pub fn start_all_services(sender: Sender<ProcessEvent>) -> Result<()> {
    let processes = clone_processes();
    if processes.len() == 0 {
        return Ok(());
    }
    for proc in processes.iter() {
        start_service(&proc.name, sender.clone())?;
    }
    Ok(())
}

pub fn start_service(service_name: &str, sender: Sender<ProcessEvent>) -> Result<()> {
    let proc_runtime = find_readonly_proc_runtime(service_name)?;
    let conf = proc_runtime.config;
    let pid = proc_runtime.pid;
    let svc_name = service_name.to_string();
    if pid.is_some() {
        let pid_val = pid.unwrap();
        if proc_is_running(pid_val) {
            info!(
                "{} process is already running with pid {}!",
                service_name, pid_val
            );
            send_process_event(&sender, service_name, EventType::Running, None, pid)?;
            return Ok(());
        }
    }
    thread::spawn(move || {
        if let Err(err) = spawn_proc(Arc::clone(&conf), sender) {
            error!("{} exited with error: {}", svc_name, err);
        }
    });
    Ok(())
}

pub fn stop_all_services() -> Result<()> {
    let processes = clone_processes();
    if processes.len() == 0 {
        return Ok(());
    }
    for proc in processes.iter() {
        stop_service(&proc.name)?;
    }
    Ok(())
}

pub fn stop_service(service_name: &str) -> Result<()> {
    let proc_runtime = find_readonly_proc_runtime(service_name)?;
    let pid = proc_runtime.pid;
    if pid.is_none() {
        info!("{} process is not running!", service_name);
        return Ok(());
    }
    let pid_val = pid.unwrap();
    let mut is_running = proc_is_running(pid_val);
    //更新进程的主动停止标志位
    update_proc_runtime(service_name, |p| {
        p.stopped_by_supervisor = true;
    })?;
    if !is_running {
        info!(
            "ignore stop command, {} process is not running, pid: {}!",
            service_name, pid_val
        );
        return Ok(());
    }
    info!("{} process (pid: {}) is stopping", service_name, pid_val);
    //首先尝试通过信号量的方式让进程自己退出
    if let Err(err) = terminate_process(pid_val) {
        warn!("signal {} (pid: {}) failed: {}", service_name, pid_val, err);
    }
    let start_time = Instant::now();
    let timeout_duration = Duration::from_secs(2);
    while is_running && start_time.elapsed() <= timeout_duration {
        thread::sleep(Duration::from_millis(200));
        is_running = proc_is_running(pid_val);
    }
    //如果超过规定时间进程没有退出，则强制杀掉进程
    if is_running {
        info!("{} process (pid: {}) is still running within the specified time after sending the interrupt signal, and is ready to be killed", service_name, pid_val);
        kill_process(pid_val)?;
    }
    Ok(())
}

pub fn restart_service(service_name: &str, sender: Sender<ProcessEvent>) -> Result<()> {
    stop_service(service_name)?;
    start_service(service_name, sender)?;
    Ok(())
}

pub fn service_proc_is_running(service_name: &str) -> bool {
    let proc_runtime = find_readonly_proc_runtime(service_name).unwrap();
    let pid = proc_runtime.pid;
    if pid.is_none() {
        return false;
    }
    proc_is_running(pid.unwrap())
}

fn spawn_proc(conf: Arc<ServiceConfig>, sender: Sender<ProcessEvent>) -> Result<()> {
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
    let child = cmd.spawn().map_err(|e| format!("{}", e));
    match child {
        Ok(mut child_proc) => {
            //更新进程状态为已启动
            update_proc_to_started(sender.clone(), svc_name, child_proc.id(), true)?;
            let exit_status = child_proc.wait().map_err(|e| format!("{}", e));
            match exit_status {
                Ok(status) => {
                    //进程正常退出
                    update_proc_to_stopped(
                        sender.clone(),
                        svc_name,
                        format!("exit code: {}", status.code().unwrap()).as_str(),
                        child_proc.id(),
                    )?;
                }
                Err(err) => {
                    //进程异常退出
                    update_proc_to_stopped(
                        sender.clone(),
                        svc_name,
                        err.as_str(),
                        child_proc.id(),
                    )?;
                }
            }
        }
        Err(err) => {
            return Err(Error::msg(format!("spawn process error: {}", err)));
        }
    }
    Ok(())
}

fn send_process_event(
    sender: &Sender<ProcessEvent>,
    service_name: &str,
    event_type: EventType,
    data: Option<String>,
    pid: Option<u32>,
) -> Result<()> {
    if let Err(err) = sender.send(ProcessEvent {
        service_name: service_name.to_string(),
        pid: pid,
        event_type: event_type.clone(),
        data: data,
    }) {
        return Err(Error::msg(format!(
            "send process event [{}:{:?}] error: {}",
            pid.map_or_else(|| "unknown".to_string(), |pid| pid.to_string()),
            event_type.clone(),
            err
        )));
    }
    Ok(())
}

// 更新服务进程的运行状态至启动
fn update_proc_to_started(
    sender: Sender<ProcessEvent>,
    service_name: &str,
    pid: u32,
    is_child_process: bool,
) -> Result<()> {
    update_proc_runtime(service_name, |proc| {
        proc.pid = Some(pid);
        proc.last_start_time = Some(SystemTime::now());
        proc.stopped_by_supervisor = false;
        proc.is_child_process = is_child_process;
    })?;
    fs::write(
        env::get_service_home(service_name).join("pid"),
        pid.to_string().as_bytes(),
    )
    .unwrap_or_else(|e| error!("{} create pid file failed:{}", service_name, e));
    send_process_event(&sender, service_name, EventType::Running, None, Some(pid))?;
    Ok(())
}

// 更新服务进程的运行状态至停止
fn update_proc_to_stopped(
    sender: Sender<ProcessEvent>,
    service_name: &str,
    exit_msg: &str,
    pid: u32,
) -> Result<()> {
    update_proc_runtime(service_name, |proc| {
        proc.pid = None;
        proc.last_stop_time = Some(SystemTime::now());
        proc.exit_err = Some(exit_msg.to_string());
    })?;
    fs::remove_file(env::get_service_home(service_name).join("pid"))
        .unwrap_or_else(|e| warn!("{} remove pid file failed:{}", service_name, e));
    let proc_info = find_readonly_proc_runtime(service_name)?;
    let event_type = if proc_info.stopped_by_supervisor {
        EventType::Stopped
    } else {
        EventType::Exited
    };
    send_process_event(
        &sender,
        service_name,
        event_type.clone(),
        Some(exit_msg.to_string()),
        Some(pid),
    )?;
    Ok(())
}

fn find_proc_from_pid_file(service_config: Arc<ServiceConfig>) -> Option<ProcessRuntimeInfo> {
    let pid_path = env::get_service_home(&service_config.name).join("pid");
    if pid_path.exists() {
        let pid_str = fs::read_to_string(pid_path).unwrap();
        let pid = pid_str.trim().parse::<u32>().unwrap();
        if proc_is_running(pid) {
            return Some(ProcessRuntimeInfo {
                name: service_config.name.clone(),
                pid: Some(pid),
                is_child_process: false,
                config: service_config,
                stopped_by_supervisor: false,
                last_start_time: Some(SystemTime::now()),
                last_stop_time: None,
                exit_err: None,
            });
        }
    }
    None
}

fn clone_processes() -> Vec<ProcessRuntimeInfo> {
    let mut processes_clone: Vec<ProcessRuntimeInfo> = Vec::new();
    let processes = PROCESSES.read().unwrap();
    for (_, process) in processes.iter().enumerate() {
        let process = process.read().unwrap();
        processes_clone.push(process.clone());
    }
    return processes_clone;
}

fn find_readonly_proc_runtime(name: &str) -> Result<ProcessRuntimeInfo> {
    let processes = PROCESSES.read().unwrap();
    for (_, process) in processes.iter().enumerate() {
        let process = process.read().unwrap();
        if process.name == name {
            return Ok(process.clone());
        }
    }
    Err(Error::msg(format!(
        "can not find process config for service:{}",
        name
    )))
}

fn update_proc_runtime<F>(service_name: &str, update_func: F) -> Result<()>
where
    F: Fn(&mut ProcessRuntimeInfo),
{
    let mut processes = PROCESSES.write().unwrap();
    if let Some(proc) = processes
        .iter_mut()
        .find(|p| p.read().unwrap().name == service_name)
    {
        let mut proc = proc.write().unwrap();
        update_func(&mut proc);
        Ok(())
    } else {
        Err(Error::msg(format!(
            "can not find process config for service:{}",
            service_name
        )))
    }
}

fn proc_is_running(pid: u32) -> bool {
    let pid = Pid::from(pid as usize);
    let mut s = System::new();
    s.refresh_processes();
    s.process(pid).is_some()
}

#[cfg(test)]
mod tests {
    use log::LevelFilter;
    use log4rs::{
        append::console::{ConsoleAppender, Target},
        config::{Appender, Root},
        Config,
    };

    use crate::config;

    use super::*;
    use std::sync::mpsc;

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
            startup_delay: Some(10),
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
        let config = mock_config();
        let orders = config.services.keys().cloned().collect();
        let service_name = "service1";
        init_processes(&config, orders).unwrap();
        assert!(start_service(service_name, sender.clone()).is_ok());
        // Check if the event was sent
        let event = receiver.recv().unwrap();
        assert_eq!(event.service_name, service_name);
        assert_eq!(event.pid.is_some(), true);
        assert_eq!(event.event_type, EventType::Running);
        thread::sleep(Duration::from_millis(100));
        let service_info = find_readonly_proc_runtime(service_name).unwrap();
        assert_eq!(service_info.pid.is_some(), true);
        assert_eq!(service_info.is_child_process, true);
        assert_eq!(service_info.last_start_time.is_some(), true);
    }

    #[test]
    fn test_stop_service() {
        init_test_log();
        let (sender, receiver) = mpsc::channel();
        let config = mock_config();
        let orders = config.services.keys().cloned().collect();
        let service_name = "service1";
        init_processes(&config, orders).unwrap();
        start_service(service_name, sender.clone()).unwrap();
        let _ = receiver.recv().unwrap();
        stop_service(service_name).unwrap();
        let stop_event = receiver.recv().unwrap();
        assert_eq!(stop_event.service_name, service_name);
        assert_eq!(stop_event.pid.is_some(), true);
        assert_eq!(stop_event.event_type, EventType::Stopped);
        thread::sleep(Duration::from_millis(100));
        let service_info = find_readonly_proc_runtime(service_name).unwrap();
        assert_eq!(service_info.pid.is_none(), true);
        assert_eq!(service_info.last_stop_time.is_some(), true);
        assert_eq!(service_info.stopped_by_supervisor, true);
    }
}
