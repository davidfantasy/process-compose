use anyhow::{Error, Result};
use clap::Parser;
use env::Args;
use log::{error, info, warn};
use process::manager::{EventType, ProcessEvent};
use std::{
    process::exit,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::Duration,
};
use sys_service::{control::control, manager::SysServiceProgram};

use crate::config::{analyze_service_dependencies, load_config};

mod config;
mod env;
mod health;
mod logger;
mod process;
mod sys_service;

fn main() {
    logger::init_log();
    load_config()
        .map_err(|e| Error::msg(format!("Failed to load config: {}", e)))
        .unwrap();
    let args = Args::parse();
    if args.service_action.is_some() {
        let action = args.service_action.unwrap();
        if let Err(err) = control(&action) {
            error!("service action {:?} failed: {}", action, err);
        } else {
            info!("{:?} successed!", action);
        }
        return;
    }
    //通过启动命令携带的参数判断是否以服务方式启动
    if env::is_run_as_service() {
        info!("Starting Process Compose as Service");
        if let Err(e) = sys_service::manager::run(Box::new(Program {})) {
            error!("process-manager service start failed: {}", e);
        }
    } else {
        if let Err(e) = run() {
            error!("process-manager run failed: {}", e);
        }
        wait_for_signal();
    }
}

struct Program {}

impl SysServiceProgram for Program {
    fn start(&self) -> anyhow::Result<()> {
        run()?;
        Ok(())
    }

    fn stop(&self) -> anyhow::Result<()> {
        process::manager::stop_all_services()?;
        Ok(())
    }
}

fn run() -> Result<()> {
    info!("process-compose starting...");
    let config = config::current_config();
    let services_congfig = config.services.values().cloned().collect();
    let services_ordered = analyze_service_dependencies(&services_congfig)?;
    process::manager::init_processes(&config, services_ordered)?;
    env::create_services_home(&services_congfig)
        .unwrap_or_else(|e| error!("create service home failed: {}", e));
    //注册服务事件处理器，并启动配置的服务
    let (tx, rx) = mpsc::channel::<ProcessEvent>();
    let msg_sender = tx.clone();
    thread::spawn(move || {
        handle_process_event(msg_sender, rx);
    });
    process::manager::start_all_services(tx.clone())
        .unwrap_or_else(|e| error!("start service failed: {}", e));
    Ok(())
}

fn wait_for_signal() {
    let term = Arc::new(AtomicBool::new(false));
    let term_clone = Arc::clone(&term);
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term_clone)).unwrap();
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term_clone)).unwrap();
    while !term_clone.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(1));
    }
    info!("received a terminate signal,try to stop all services...");
    process::manager::stop_all_services().unwrap_or_else(|e| error!("stop service failed: {}", e));
    exit(0);
}

fn handle_process_event(sender: Sender<ProcessEvent>, rx: Receiver<ProcessEvent>) {
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
                health::start_watch(
                    received.service_name,
                    service_cfg.unwrap().healthcheck,
                    sender.clone(),
                )
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
        }
    }
}
