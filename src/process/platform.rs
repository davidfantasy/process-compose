#[cfg(target_os = "windows")]
pub mod windows {
    use anyhow::{Error, Result};
    use encoding::all::{GB18030, UTF_8};
    use encoding::{DecoderTrap, Encoding};
    use std::{os::windows::process::CommandExt, process::Command};
    use winapi::shared::minwindef::FALSE;
    use winapi::um::consoleapi::SetConsoleCtrlHandler;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::winbase::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    };
    use winapi::um::wincon::{
        AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT, CTRL_C_EVENT,
    };

    use crate::env::is_run_as_service;

    pub fn before_exec(cmd: &mut Command) -> Result<()> {
        cmd.creation_flags(CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP);
        Ok(())
    }

    pub fn terminate_process(pid: u32) -> Result<()> {
        //如果不指定/F参数，无法终止某个进程的子进程，而process-compose的所有服务都是子进程，所以这里无法
        //使用taskkill的方式来进行信号通知
        //kill_proc(pid, false)?;
        signal_interupt(pid)?;
        Ok(())
    }

    pub fn kill_process(pid: u32) -> Result<()> {
        kill_proc(pid, true)?;
        Ok(())
    }

    fn kill_proc(pid: u32, force: bool) -> Result<()> {
        let mut kill_cmd = Command::new("taskkill.exe");
        let mut args = vec![];
        if force {
            args.push("/F");
        }
        args.push("/T");
        args.push("/PID");
        let pid_str = pid.to_string();
        args.push(pid_str.as_str());
        kill_cmd.args(&args);
        kill_cmd.creation_flags(CREATE_NO_WINDOW);
        let output = kill_cmd.output()?;
        if !output.status.success() {
            let stdout = decode_msg(output.stdout);
            let stderr = decode_msg(output.stderr);
            let mut err_msg = if stderr.is_empty() { stdout } else { stderr };
            if err_msg.is_empty() {
                err_msg = "unkown error".to_string();
            }
            return Err(Error::msg(format!(
                "An error occurred when attempting to terminate process：{}",
                err_msg
            )));
        }
        Ok(())
    }

    //模拟CTRL_BREAK和CTRL_C信息向受管进程发送,缺陷也是无法照顾到子进程
    fn signal_interupt(pid: u32) -> Result<()> {
        unsafe {
            //仅仅在服务模式下才需要附加到控制台，这样才能确保后续模拟事件的正确发送
            if is_run_as_service() {
                if FreeConsole() == FALSE {
                    let err = GetLastError();
                    return Err(Error::msg(format!("FreeConsole failed {}", err)));
                }
                if AttachConsole(pid) == FALSE {
                    let err = GetLastError();
                    return Err(Error::msg(format!("AttachConsole failed {}", err)));
                }
                //TODO:参数是否正确？
                if SetConsoleCtrlHandler(None, 1) == 0 {
                    let err = GetLastError();
                    return Err(Error::msg(format!("SetConsoleCtrlHandler failed {}", err)));
                }
            }
            // 生成模拟的CTRL_BREAK_EVENT和CTRL_C_EVENT事件,注意generateconsolectrlevent的第二个参数是进程组id
            // 需要确保创建的进程使用了CREATE_NEW_PROCESS_GROUP标识，这样进程ID等于其进程组ID，否则无法基于pid发送信号
            if GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) == FALSE {
                let err = GetLastError();
                return Err(Error::msg(format!("Send CTRL_BREAK_EVENT failed {}", err)));
            }
            if GenerateConsoleCtrlEvent(CTRL_C_EVENT, pid) == FALSE {
                return Err(Error::msg("Send CTRL_C_EVENT failed"));
            }
        }
        Ok(())
    }

    fn decode_msg(bytes: Vec<u8>) -> String {
        let mut r = UTF_8.decode(&bytes, DecoderTrap::Strict);
        if r.is_err() {
            r = GB18030.decode(&bytes, DecoderTrap::Strict);
        }
        return r.unwrap_or("".to_string());
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::process::Command;
        use std::thread;
        use std::time::Duration;

        #[test]
        fn test_kill_proc() {
            //创建一个新的进程
            let mut child = Command::new("timeout.exe")
                .arg("/t")
                .arg("10")
                .creation_flags(CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP)
                .spawn()
                .expect("Failed to start new process");
            // 获取新进程的 PID
            let pid = child.id();
            println!("Started new process with PID: {}", pid);
            // // 等待一段时间以确保新进程已经启动
            thread::sleep(Duration::from_secs(1));
            // 尝试杀死新进程
            match terminate_process(pid) {
                Ok(_) => println!("Successfully killed process with PID: {}", pid),
                Err(e) => println!("Failed to kill process with PID: {}. Error: {}", pid, e),
            };
            //确保子进程已经结束
            let _ = child.wait();
        }
    }
}

#[cfg(target_os = "linux")]
pub mod linux {

    use anyhow::anyhow;
    use anyhow::Result;
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use nix::unistd::{getpgid, setpgid};
    use std::convert::TryInto;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    pub fn before_exec(cmd: &mut Command) -> Result<()> {
        // 在 Unix 平台上，设置新进程的进程组ID与其进程ID相同，这样它就会成为新的进程组的领导者。
        unsafe {
            cmd.pre_exec(|| match setpgid(Pid::from_raw(0), Pid::from_raw(0)) {
                Ok(_) => Ok(()),
                Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, e)),
            });
        }
        Ok(())
    }

    pub fn terminate_process(pid: u32) -> Result<()> {
        pid.try_into()
            .map_err(|_| anyhow!("PID out of range"))
            .and_then(|pid| signal_proc(pid, Signal::SIGTERM))
    }

    pub fn kill_process(pid: u32) -> Result<()> {
        pid.try_into()
            .map_err(|_| anyhow!("PID out of range"))
            .and_then(|pid| signal_proc(pid, Signal::SIGKILL))
    }

    fn signal_proc(pid: i32, signal: Signal) -> Result<()> {
        let pgid = getpgid(Some(Pid::from_raw(pid)))?;
        // 如果进程是当前的进程组长，则通过指定负数的pid向整个进程组发送信号
        let pid = if pgid == Pid::from_raw(pid) {
            Pid::from_raw(-pid)
        } else {
            Pid::from_raw(pid)
        };
        kill(pid, signal).map_err(|e| {
            anyhow!(
                "failed to signal process with signal {:?} to pid {}: {}",
                signal,
                pid,
                e
            )
        })?;
        Ok(())
    }
}
