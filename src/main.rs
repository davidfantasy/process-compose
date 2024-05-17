use anyhow::{Error, Result};
use clap::Parser;
use env::Args;
use log::{error, info};
use std::{
    process::exit,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self},
        Arc,
    },
    thread,
    time::Duration,
};
use sys_service::{control::control, manager::SysServiceProgram};

use crate::{
    config::{analyze_service_dependencies, load_config},
    event::ProcessEvent,
};

mod config;
mod env;
mod event;
mod health;
mod logger;
mod process;
mod sys_service;

fn main() {
    //先以默认等级初始化日志框架，避免初始化配置时的信息无法输出
    logger::init_log("");
    load_config()
        .map_err(|e| Error::msg(format!("Failed to load config: {}", e)))
        .unwrap();
    logger::change_log_level(config::current_config().log_level.as_str());
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
        let all_services = process::status::get_all_process_name();
        process::manager::stop_services(all_services)?;
        Ok(())
    }
}

fn run() -> Result<()> {
    info!("process-compose starting...");
    let config = config::current_config();
    let services_congfig = config.services.values().cloned().collect();
    let services_ordered = analyze_service_dependencies(&services_congfig)?;
    process::status::init_processes(&config, services_ordered)?;
    env::create_services_home(&services_congfig)
        .unwrap_or_else(|e| error!("create service home failed: {}", e));
    //注册服务事件处理器，并启动配置的服务
    let (tx, rx) = mpsc::channel::<ProcessEvent>();
    thread::spawn(move || {
        event::handle_process_event(tx, rx);
    });
    let all_services = process::status::get_all_process_name();
    process::manager::start_services(all_services)
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
    let all_services = process::status::get_all_process_name();
    process::manager::stop_services(all_services)
        .unwrap_or_else(|e| error!("stop service failed: {}", e));
    exit(0);
}
