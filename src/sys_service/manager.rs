use anyhow::{Ok, Result};
use lazy_static::lazy_static;

#[cfg(target_os = "linux")]
use super::linux_service::LinuxSysService as PlatformSysService;
#[cfg(target_os = "windows")]
use super::windows_service::WindowsSysService as PlatformSysService;

lazy_static! {
    static ref SYS_SERVICE: Box<dyn SysService> = Box::new(PlatformSysService::new());
}

pub(crate) trait SysService: Send + Sync {
    fn run(&self, program: Box<dyn SysServiceProgram>) -> Result<()>;
    fn install(&self) -> Result<()>;
}

pub trait SysServiceProgram: Send + Sync {
    fn start(&self) -> Result<()>;
    fn stop(&self) -> Result<()>;
}

pub fn run(program: Box<dyn SysServiceProgram>) -> Result<()> {
    SYS_SERVICE.run(program)?;
    Ok(())
}

pub fn install() -> Result<()> {
    SYS_SERVICE.install()?;
    Ok(())
}
