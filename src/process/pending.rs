use std::sync::{mpsc::Sender, RwLock};

use anyhow::{Error, Result};
use log::{error, info};

use crate::event::ProcessEvent;

use super::{manager, status};

#[derive(Clone, Debug)]
struct ProcessStartInfo {
    name: String,
    depends: Vec<String>,
}

static PENDING_SERVICES: RwLock<Vec<RwLock<ProcessStartInfo>>> = RwLock::new(Vec::new());

pub(crate) fn add_pending_service(name: &str, depends: Vec<String>) {
    let mut pending_list = PENDING_SERVICES.write().unwrap();
    pending_list.push(RwLock::new(ProcessStartInfo {
        name: name.to_owned(),
        depends: depends,
    }));
}

pub(crate) fn remove_pending_service(name: &str) {
    let mut pending_list = PENDING_SERVICES.write().unwrap();
    pending_list.retain(|s| s.read().unwrap().name != name);
}

pub fn try_start_pending_service(name: &str, sender: Sender<ProcessEvent>) {
    let pending_service = find_readonly_pending_info(name);
    if let Some(pending_service) = pending_service {
        let mut met = true;
        for dep in pending_service.depends {
            let health = status::is_heathy(&dep);
            if health.is_none() || !health.unwrap() {
                met = false;
                break;
            }
        }
        if met {
            info!("startup dependency conditions for {} have been met", name);
            manager::start_service(name, sender).unwrap_or_else(|err| {
                error!("start service {} failed: {}", name, err);
            });
            //启动任务并删除待启动列表
            remove_pending_service(name);
        }
    }
}

fn find_readonly_pending_info(name: &str) -> Option<ProcessStartInfo> {
    let pending_list = PENDING_SERVICES.read().unwrap();
    for (_, service) in pending_list.iter().enumerate() {
        let service = service.read().unwrap();
        if service.name == name {
            return Some(service.clone());
        }
    }
    None
}
