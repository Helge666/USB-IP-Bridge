//! Windows service integration (Service Control Manager).

use crate::engine::Engine;
use anyhow::{Context, Result};
use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceDependency, ServiceActionType, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

pub const SERVICE_NAME: &str = "USB-IP-Bridge";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

define_windows_service!(ffi_service_main, service_main);

pub fn dispatch() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).context("service dispatcher")?;
    Ok(())
}

fn log_path() -> std::path::PathBuf {
    usbipb_core::config::config_dir().join("logs").join("service.log")
}

fn service_main(_args: Vec<OsString>) {
    usbipb_core::logging::init(Some(&log_path()));
    if let Err(e) = run_service() {
        log::error!("service failed: {e:#}");
    }
}

fn run_service() -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let handler = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    let set = |state, accept, wait| {
        handler.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: wait,
            process_id: None,
        })
    };
    set(ServiceState::StartPending, ServiceControlAccept::empty(), Duration::from_secs(10))?;
    let engine = Engine::start()?;
    set(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN, Duration::default())?;
    let _ = rx.recv();
    set(ServiceState::StopPending, ServiceControlAccept::empty(), Duration::from_secs(15))?;
    engine.shutdown();
    set(ServiceState::Stopped, ServiceControlAccept::empty(), Duration::default())?;
    Ok(())
}

pub fn install() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .context("open service manager (run as administrator)")?;
    let exe = std::env::current_exe()?;
    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: "USB-IP-Bridge".into(),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec!["service".into()],
        // The VBoxUSB monitor driver (from usbipd-win) is started on demand; Windows starts it
        // before this service.
        dependencies: vec![ServiceDependency::Service("VBoxUsbMon".into())],
        account_name: None, // LocalSystem
        account_password: None,
    };
    let access = ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS;
    // Installing again (an update) brings the existing service up to date.
    let service = match manager.open_service(SERVICE_NAME, access) {
        Ok(s) => {
            s.change_config(&info)?;
            s
        }
        Err(_) => manager.create_service(&info, access)?,
    };
    service.set_description("Shares USB devices with other computers over the network (USB/IP).")?;
    // Restart automatically if the service ever crashes.
    let restart = ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) };
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart.clone(), restart.clone(), restart]),
    })?;
    if service.query_status()?.current_state == ServiceState::Stopped {
        service.start::<&str>(&[])?;
    }
    println!("service '{SERVICE_NAME}' installed and started");
    Ok(())
}

/// Stops the service and waits until it has stopped (before an update replaces it).
pub fn stop() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).context("open service manager (run as administrator)")?;
    let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP) else { return Ok(()) };
    stop_and_wait(&service)
}

fn stop_and_wait(service: &windows_service::service::Service) -> Result<()> {
    if service.query_status()?.current_state != ServiceState::Stopped {
        let _ = service.stop();
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(500));
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
        }
    }
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).context("open service manager (run as administrator)")?;
    let Ok(service) = manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE) else {
        println!("service '{SERVICE_NAME}' is not installed");
        return Ok(());
    };
    stop_and_wait(&service)?;
    service.delete()?;
    println!("service '{SERVICE_NAME}' removed");
    Ok(())
}
