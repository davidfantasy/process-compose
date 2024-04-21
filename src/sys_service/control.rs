use anyhow::Result;
use service_manager::{
    ServiceLabel, ServiceManager, ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx,
};

use crate::config;

use super::manager::{self};

pub const RUN_AS_SERVICE_ARG: &str = "run-as-service";

pub fn control(cmd: &str) -> Result<()> {
    let current_config = config::current_config();
    let label: ServiceLabel = current_config.sys_service_name.parse().unwrap();
    let manager = <dyn ServiceManager>::native().expect("Failed to detect management platform");
    if cmd == "install" {
        manager::install()?;
    } else if cmd == "uninstall" {
        manager.uninstall(ServiceUninstallCtx {
            label: label.clone(),
        })?;
    } else if cmd == "start" {
        manager.start(ServiceStartCtx {
            label: label.clone(),
        })?;
    } else if cmd == "stop" {
        manager.stop(ServiceStopCtx {
            label: label.clone(),
        })?;
    }
    Ok(())
}
