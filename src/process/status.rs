use crate::config::{GlobalConfig, ServiceConfig};
use crate::event::{EventType, ProcessEvent};
use crate::{env, event};
use anyhow::{Error, Result};
use log::{error, info, warn};
use std::collections::HashMap;
use std::fs;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use sysinfo::{Pid, System};

#[derive(Clone, Debug)]
pub(crate) struct ProcessRuntimeInfo {
    pub(crate) name: String,
    pub(crate) pid: Option<u32>,
    pub(crate) is_child_process: bool,
    pub(crate) health: Option<bool>,
    pub(crate) config: Arc<ServiceConfig>,
    pub(crate) stopped_by_supervisor: bool,
    pub(crate) last_start_time: Option<SystemTime>,
    pub(crate) last_stop_time: Option<SystemTime>,
    pub(crate) exit_err: Option<String>,
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
                    health: None,
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

pub fn is_running_by_name(service_name: &str) -> bool {
    let proc_runtime = find_readonly_proc_runtime(service_name).unwrap();
    let pid = proc_runtime.pid;
    if pid.is_none() {
        return false;
    }
    is_running_by_pid(pid.unwrap())
}

pub fn is_running_by_pid(pid: u32) -> bool {
    let pid = Pid::from(pid as usize);
    let mut s = System::new();
    s.refresh_processes();
    s.process(pid).is_some()
}

//更新服务进程的健康状态
pub fn change_proc_health_status(name: &str, health: bool) -> Result<()> {
    update_proc_runtime(name, |proc| {
        if proc.health.is_none() || proc.health.unwrap() != health {
            info!("service [{}] health changed to {}", name, health)
        }
        proc.health = Some(health);
    })?;
    Ok(())
}

//查询某个服务的健康状态
pub fn is_heathy(name: &str) -> Option<bool> {
    let proc_runtime = find_readonly_proc_runtime(name).unwrap();
    return proc_runtime.health;
}

pub fn check_dep_ok(name: &str) -> bool {
    let service = find_readonly_proc_runtime(name);
    if service.is_err() {
        return false;
    }
    let deps = service.unwrap().config.depends_on.clone();
    if deps.is_none() {
        return true;
    }
    let deps = deps.unwrap();
    for dep in deps {
        if !is_heathy(&dep).unwrap_or(false) {
            return false;
        }
    }
    return true;
}

// 更新服务进程的运行状态至启动
pub(crate) fn update_proc_to_started(
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
    event::send_process_event(service_name, EventType::Running, None, Some(pid));
    Ok(())
}

// 更新服务进程的运行状态至停止
pub(crate) fn update_proc_to_stopped(service_name: &str, exit_msg: &str, pid: u32) -> Result<()> {
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
    event::send_process_event(
        service_name,
        event_type.clone(),
        Some(exit_msg.to_string()),
        Some(pid),
    );
    Ok(())
}

fn find_proc_from_pid_file(service_config: Arc<ServiceConfig>) -> Option<ProcessRuntimeInfo> {
    let pid_path = env::get_service_home(&service_config.name).join("pid");
    if pid_path.exists() {
        let pid_str = fs::read_to_string(pid_path).unwrap();
        let pid = pid_str.trim().parse::<u32>().unwrap();
        if is_running_by_pid(pid) {
            return Some(ProcessRuntimeInfo {
                name: service_config.name.clone(),
                pid: Some(pid),
                health: None,
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

pub fn get_all_process_name() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let processes = PROCESSES.read().unwrap();
    for (_, process) in processes.iter().enumerate() {
        let process = process.read().unwrap();
        names.push(process.name.clone());
    }
    return names;
}

pub(crate) fn find_readonly_proc_runtime(name: &str) -> Result<ProcessRuntimeInfo> {
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

pub(crate) fn update_proc_runtime<F>(service_name: &str, update_func: F) -> Result<()>
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
