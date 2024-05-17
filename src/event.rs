use std::sync::{
    mpsc::{Receiver, Sender},
    RwLock,
};

use log::{debug, error, info, warn};

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

static EVENT_SENDER: RwLock<Option<Sender<ProcessEvent>>> = RwLock::new(None);

pub fn send_process_event(
    service_name: &str,
    event_type: EventType,
    data: Option<String>,
    pid: Option<u32>,
) {
    let sender = EVENT_SENDER.read().unwrap();
    if sender.is_none() {
        error!(
            "send process event [{}:{:?}] error: sender is none",
            service_name, event_type
        );
        return;
    }
    let sender = sender.as_ref().unwrap();
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
    {
        EVENT_SENDER.write().unwrap().replace(sender);
    }
    for received in rx {
        debug!(
            "received a event:{},{:?}",
            received.service_name, received.event_type
        );
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
                health::start_watch(received.service_name, service_cfg.unwrap().healthcheck);
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
                process::status::change_proc_health_status(&received.service_name, true)
                    .unwrap_or_else(|err| {
                        warn!(
                            "change process {} health status failed: {}",
                            &received.service_name, err
                        );
                    });
                process::pending::try_start_pending_service();
            }
        }
    }
    error!("event handler has been stoped!!!!!!")
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, thread, time::Duration};

    use crate::logger;

    use super::{send_process_event, ProcessEvent};

    #[test]
    fn test() {
        logger::init_log("debug");
        let (tx, rx) = mpsc::channel::<ProcessEvent>();
        thread::spawn(move || {
            super::handle_process_event(tx, rx);
        });
        thread::sleep(Duration::from_secs(1)); // 休眠2秒
        send_process_event("test1", super::EventType::Running, None, None);
        send_process_event("test2", super::EventType::Running, None, None);
        send_process_event("test3", super::EventType::Running, None, None);
        println!("1111111111111111111111111");
        thread::sleep(Duration::from_secs(1));
    }
}
