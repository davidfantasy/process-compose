use crate::{
    config::HealthCheckConfig,
    event::{self, EventType, ProcessEvent},
    process,
};
use lazy_static::lazy_static;
use log::{info, warn};
use std::{
    collections::HashMap,
    sync::{mpsc::Sender, RwLock},
    thread,
    time::Duration,
};

lazy_static! {
    static ref SERVICES_HEALTH_STATUS: RwLock<HashMap<String, i32>> = RwLock::new(HashMap::new());
}

pub fn start_watch(
    service_name: String,
    config: Option<HealthCheckConfig>,
    sender: Sender<ProcessEvent>,
) {
    let health_cfg = config.clone(); // to avoid borrow count
    if health_cfg.is_none() || !health_cfg.unwrap().enable {
        info!("service {} is not enabled to health check", &service_name);
        return;
    }
    if is_watching(&service_name) {
        return;
    }
    set_watch_flag(&service_name);
    thread::spawn(move || do_watch_health(service_name, config.unwrap(), sender));
    return;
}

pub fn stop_watch(service_name: String) {
    let mut status = SERVICES_HEALTH_STATUS.write().unwrap();
    if !status.contains_key(&service_name) {
        warn!(
            "The service {} is not being watched, ignore stop",
            &service_name
        );
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

fn do_watch_health(service_name: String, config: HealthCheckConfig, sender: Sender<ProcessEvent>) {
    if config.check_delay.is_some() {
        thread::sleep(Duration::from_secs(config.check_delay.unwrap() as u64));
    }
    if !is_watching(&service_name) {
        return;
    }
    info!("The service {} has enabled health checks", &service_name);
    loop {
        if !is_watching(&service_name) {
            info!(
                "The service {} is not being watched, stop health check",
                &service_name
            );
            break;
        }
        let success = check(&service_name, &config);
        let mut check_interval = config.interval;
        if check_interval < 5 {
            check_interval = 5;
        }
        let tx = sender.clone();
        if !success {
            event::send_process_event(tx, &service_name, EventType::Unhealthy, None, None);
            let fail_times = incr_fail_times(&service_name);
            let restart: bool = fail_times > config.max_failures;
            if restart {
                warn!("The health check failure count for service {} has exceeded the threshold, preparing to restart it", &service_name);
                process::manager::restart_service(&service_name, sender.clone()).unwrap_or_else(
                    |err| {
                        warn!("restart service {} failed: {}", &service_name, err);
                    },
                );
                check_interval += config.check_delay.unwrap_or(0);
            }
        } else {
            event::send_process_event(tx, &service_name, EventType::Healthy, None, None);
        }
        thread::sleep(Duration::from_secs(check_interval as u64));
    }
}

fn check(service_name: &str, config: &HealthCheckConfig) -> bool {
    if config.url.is_none() {
        return test_with_process(service_name);
    }
    test_with_http(&config.url.clone().unwrap())
}

fn incr_fail_times(service_name: &str) -> i32 {
    let mut service_statuses = SERVICES_HEALTH_STATUS.write().unwrap();
    let fail_times = service_statuses.entry(service_name.to_owned()).or_insert(0);
    *fail_times += 1;
    *fail_times
}

fn test_with_process(service_name: &str) -> bool {
    process::status::is_running_by_name(service_name)
}

fn test_with_http(url: &str) -> bool {
    let req = reqwest::blocking::get(url);
    if req.is_err() {
        return false;
    }
    let status = req.unwrap().status();
    status.is_success()
}
