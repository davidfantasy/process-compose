use std::sync::mpsc::{Receiver, Sender};

use anyhow::{Error, Result};
use log::{error, info, warn};

use crate::{config, health, process};

#[derive(Debug, Clone, PartialEq)]
pub enum EventType {
    //进程运行成功
    Running = 1,
    //进程被compose主动停止
    Stopped = 2,
    //进程自身退出
    Exited = 3,
    //健康检查没通过
    Unhealthy = 4,
    //健康检查通过
    Healthy = 5,
}

pub struct ProcessEvent {
    pub service_name: String,
    pub pid: Option<u32>,
    pub event_type: EventType,
    pub data: Option<String>,
}

pub fn send_process_event(
    sender: Sender<ProcessEvent>,
    service_name: &str,
    event_type: EventType,
    data: Option<String>,
    pid: Option<u32>,
) {
    if let Err(err) = sender.send(ProcessEvent {
        service_name: service_name.to_string(),
        pid: pid,
        event_type: event_type.clone(),
        data: data,
    }) {
        error!(
            "send process event [{}:{:?}] error: {}",
            service_name, event_type, err
        );
    }
}

pub fn handle_process_event(sender: Sender<ProcessEvent>, rx: Receiver<ProcessEvent>) {
    for received in rx {
        match received.event_type {
            EventType::Running => {
                info!(
                    "The {} service started with pid: {}",
                    received.service_name,
                    received
                        .pid
                        .map_or_else(|| "unknown".to_string(), |pid| pid.to_string())
                );
                let service_cfg = config::find_service_config(&received.service_name);
                let tx = sender.clone();
                health::start_watch(received.service_name, service_cfg.unwrap().healthcheck, tx)
            }
            EventType::Exited => {
                let pid = received
                    .pid
                    .map_or_else(|| "unknown".to_string(), |pid| pid.to_string());
                let msg = received.data.or(Some("unknown".to_string())).unwrap();
                warn!(
                    "The {} service (pid: {}) has exited:{}",
                    received.service_name, pid, msg
                );
            }
            EventType::Stopped => {
                let pid = received
                    .pid
                    .map_or_else(|| "unknown".to_string(), |pid| pid.to_string());
                info!(
                    "The {} service (pid: {}) has be stopped,will stop health watch",
                    received.service_name, pid,
                );
                health::stop_watch(received.service_name)
            }
            EventType::Unhealthy => {
                process::status::change_proc_health_status(&received.service_name, false)
                    .unwrap_or_else(|err| {
                        warn!(
                            "change process {} health status failed: {}",
                            &received.service_name, err
                        );
                    });
            }
            EventType::Healthy => {
                let tx = sender.clone();
                process::status::change_proc_health_status(&received.service_name, true)
                    .unwrap_or_else(|err| {
                        warn!(
                            "change process {} health status failed: {}",
                            &received.service_name, err
                        );
                    });
                process::pending::try_start_pending_service(&received.service_name, tx)
            }
        }
    }
}
