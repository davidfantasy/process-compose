use std::sync::RwLock;

use log::{debug, error, info};

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

fn remove_pending_service(name: &str) {
    let mut pending_list = PENDING_SERVICES.write().unwrap();
    pending_list.retain(|s| s.read().unwrap().name != name);
}

pub fn try_start_pending_service() {
    let pending_list = PENDING_SERVICES.read().unwrap();
    let mut started_services: Vec<String> = Vec::new();
    for (_, service) in pending_list.iter().enumerate() {
        let pending_service = service.read().unwrap();
        let mut met = true;
        let name = &pending_service.name;
        for dep in &pending_service.depends {
            let health = status::is_heathy(&dep);
            if health.is_none() || !health.unwrap() {
                met = false;
                break;
            }
        }
        if met {
            info!("startup dependency conditions for {} have been met", name);
            manager::start_service(&name).unwrap_or_else(|err| {
                error!("start service {} failed: {}", name, err);
            });
            started_services.push(pending_service.name.clone());
        }
    }
    drop(pending_list);
    //已启动的任务需要从待启动列表中删除
    for name in started_services {
        remove_pending_service(&name);
    }
}
