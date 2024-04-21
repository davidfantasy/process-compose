use std::{
    ffi::OsString,
    sync::{mpsc, Mutex},
    time::Duration,
};

use crate::config;

use super::{
    control::RUN_AS_SERVICE_ARG,
    manager::{SysService, SysServiceProgram},
};
use anyhow::Result;
use lazy_static::lazy_static;
use log::{error, info};
use windows_service::{
    define_windows_service,
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

define_windows_service!(ffi_service_main, sys_service_main);

const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

lazy_static! {
    static ref PROGRAM: Mutex<Option<Box<dyn SysServiceProgram>>> = Mutex::new(None);
}

pub(crate) struct WindowsSysService {}

impl WindowsSysService {
    pub fn new() -> Self {
        WindowsSysService {}
    }
}

impl SysService for WindowsSysService {
    fn run(&self, program: Box<dyn SysServiceProgram>) -> Result<()> {
        let current_config = config::current_config();
        let service_name = current_config.sys_service_name;
        let mut global_program = PROGRAM.lock().unwrap();
        *global_program = Some(program);
        drop(global_program);
        info!("Starting Process Compose as Windows Service");
        service_dispatcher::start(service_name, ffi_service_main)?;
        Ok(())
    }

    fn install(&self) -> Result<()> {
        let current_config = config::current_config();
        let service_name = current_config.sys_service_name;
        let service_desc = current_config.sys_service_desc;
        let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
        let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)?;
        let service_info = ServiceInfo {
            name: OsString::from(service_name.clone()),
            display_name: OsString::from(service_name.clone()),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: std::env::current_exe().unwrap(),
            launch_arguments: vec![format!("--{}", RUN_AS_SERVICE_ARG).into()],
            dependencies: vec![],
            account_name: None, // run as System
            account_password: None,
        };
        let service = service_manager.create_service(
            &service_info,
            ServiceAccess::START | ServiceAccess::CHANGE_CONFIG,
        )?;
        //服务描述
        service.set_description(service_desc)?;
        //配置服务失败后的重启策略
        let service_actions = vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(10),
            },
            ServiceAction {
                action_type: ServiceActionType::None,
                delay: Duration::default(),
            },
        ];
        let failure_actions = ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86400)),
            reboot_msg: None,
            command: None,
            actions: Some(service_actions),
        };
        service.update_failure_actions(failure_actions)?;
        Ok(())
    }
}

fn run_service() -> Result<()> {
    let current_config = config::current_config();
    let binding = PROGRAM.lock().unwrap();
    let program = binding.as_ref().unwrap();
    program.start()?;
    // Create a channel to be able to poll a stop event from the service worker loop.
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    // Define system service event handler that will be receiving service events.
    let event_handler = {
        //let shutdown_tx = shutdown_tx.clone();
        move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                // Notifies a service to report its current status information to the service
                // control manager. Always return NoError even if not implemented.
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                // Handle stop
                ServiceControl::Stop => {
                    shutdown_tx.send(()).unwrap();
                    ServiceControlHandlerResult::NoError
                }
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        }
    };
    let status_handle =
        service_control_handler::register(current_config.sys_service_name, event_handler)?;
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    loop {
        match shutdown_rx.recv_timeout(Duration::from_millis(100)) {
            // Break the loop either upon stop or channel disconnect
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            // Continue work if no events were received within the timeout
            Err(mpsc::RecvTimeoutError::Timeout) => (),
        };
    }
    info!("received stop event from service control manager,stopping all services...");
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    if let Err(err) = program.stop() {
        error!("Error stopping service:{:?}", err);
    }
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;
    Ok(())
}

fn sys_service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        error!("main thread failed:{:?}", e);
    }
}
