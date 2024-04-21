use std::{
    env,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use log::info;
use service_manager::{ServiceInstallCtx, ServiceManager};

use anyhow::{Ok, Result};

use crate::{
    config,
    sys_service::{
        control::RUN_AS_SERVICE_ARG,
        manager::{SysService, SysServiceProgram},
    },
};

pub(crate) struct LinuxSysService {}

impl LinuxSysService {
    pub fn new() -> Self {
        LinuxSysService {}
    }
}

impl SysService for LinuxSysService {
    fn run(&self, program: Box<dyn SysServiceProgram>) -> Result<()> {
        program.start()?;
        wait_for_signal();
        program.stop()?;
        Ok(())
    }

    fn install(&self) -> Result<()> {
        let manager = <dyn ServiceManager>::native().expect("Failed to detect management platform");
        manager.install(ServiceInstallCtx {
            label: config::current_config().sys_service_name.parse().unwrap(),
            program: env::current_exe().unwrap(),
            args: vec![format!("--{}", RUN_AS_SERVICE_ARG).into()],
            contents: None,
            username: None,
            working_directory: None,
            environment: None,
        })?;
        Ok(())
    }
}

fn wait_for_signal() {
    let term = Arc::new(AtomicBool::new(false));
    let term_clone = Arc::clone(&term);
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term_clone)).unwrap();
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&term_clone)).unwrap();
    while !term_clone.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_secs(1));
    }
    info!("received a terminate signal");
}
