use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::Result;
use chrono::Utc;
use clap::Parser;
use lazy_static::lazy_static;

use crate::config::{self, ServiceConfig};

lazy_static! {
    pub static ref ROOT_DIR: PathBuf = {
        let path = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        path
    };
}

pub fn is_run_as_service() -> bool {
    Args::parse().run_as_service
}

pub fn create_services_home(services: &Vec<ServiceConfig>) -> Result<()> {
    for service in services {
        let dir = get_service_log_dir(&service.name);
        fs::create_dir_all(dir)?;
        let dir = get_service_data_dir(&service.name);
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn create_service_redirect_log_file(svc_name: &str, file_prefix: &str) -> Result<File> {
    let dir = get_service_log_dir(svc_name);
    fs::create_dir_all(&dir)?;
    let today = Utc::now().format("%Y%m%d").to_string();
    let file_path = format!("{}/{}_{}.log", dir.to_string_lossy(), file_prefix, today);
    let file = File::create(&file_path)?;
    Ok(file)
}

pub fn get_service_home(service_name: &str) -> PathBuf {
    let config = config::current_config();
    Path::new(&config.app_data_home).join(service_name)
}

fn get_service_log_dir(svc_name: &str) -> PathBuf {
    get_service_home(svc_name).join("logs")
}

fn get_service_data_dir(svc_name: &str) -> PathBuf {
    get_service_home(svc_name).join("data")
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// service action, support: start, stop, install, uninstall
    pub service_action: Option<String>,

    /// internal arg,don't use it
    #[arg(long, default_value_t = false)]
    pub run_as_service: bool,
}
