use std::ffi::OsString;
use std::fmt;
use std::mem;
use std::os::windows::ffi::OsStringExt;

use ntapi::ntrtl;
use winapi::shared::{minwindef, ntstatus};
use winapi::um::{sysinfoapi, winbase, winnt};

use heim_common::prelude::{Error, Result};

use crate::Arch;

// TODO: Is not declared in `winapi` crate
// See https://github.com/retep998/winapi-rs/issues/780
const MAX_COMPUTERNAME_LENGTH: minwindef::DWORD = 31;

// Partial copy of the `sysinfoapi::SYSTEM_INFO`,
// because it contains pointers and we need to sent it between threads.
// TODO: It would be better to make `SYSTEM_INFO` Sendable somehow?
#[derive(Debug)]
struct SystemInfo {
    processor_arch: minwindef::WORD,
}

impl From<sysinfoapi::SYSTEM_INFO> for SystemInfo {
    fn from(info: sysinfoapi::SYSTEM_INFO) -> SystemInfo {
        let s = unsafe { info.u.s() };

        SystemInfo {
            processor_arch: s.wProcessorArchitecture,
        }
    }
}

pub struct Platform {
    sysinfo: SystemInfo,
    version: winnt::OSVERSIONINFOEXW,
    hostname: String,
    build: String,
}

impl Platform {
    pub fn system(&self) -> &str {
        match self.version.wProductType {
            winnt::VER_NT_WORKSTATION => "Windows",
            winnt::VER_NT_SERVER => "Windows Server",
            winut::VER_NT_DOMAIN_CONTROLLER => "Windows Server",
            other => unreachable!("Unknown Windows product type: {}", other),
        }
    }

    pub fn release(&self) -> &str {
        // https://docs.microsoft.com/en-us/windows-hardware/drivers/ddi/content/wdm/ns-wdm-_osversioninfoexw#remarks

        let major = self.version.dwMajorVersion;
        let minor = self.version.dwMinorVersion;
        let suite_mask = minwindef::DWORD::from(self.version.wSuiteMask);
        let is_workstation = self.version.wProductType == winnt::VER_NT_WORKSTATION;

        match (major, minor) {
            (10, 0) if is_workstation => "10",
            (10, 0) if !is_workstation => "2016",
            (6, 3) if is_workstation => "8.1",
            (6, 3) if !is_workstation => "2012 R2",
            (6, 2) if is_workstation => "8",
            (6, 2) if !is_workstation => "2012",
            (6, 1) if is_workstation => "7",
            (6, 1) if !is_workstation => "2008 R2",
            (6, 0) if is_workstation => "Vista",
            (6, 0) if !is_workstation => "2008",
            (5, 2) if suite_mask == winnt::VER_SUITE_WH_SERVER => "Home Server",
            (5, 2) if is_workstation => "XP Professional x64 Edition",
            (5, 2) => "2003",
            (5, 1) => "XP",
            (5, 0) => "2000",
            _ => "unknown",
        }
    }

    pub fn version(&self) -> &str {
        self.build.as_str()
    }

    pub fn hostname(&self) -> &str {
        self.hostname.as_str()
    }

    pub fn architecture(&self) -> Arch {
        match self.sysinfo.processor_arch {
            // While there are other `PROCESSOR_ARCHITECTURE_*` consts exists,
            // MSDN described only the following.
            // https://docs.microsoft.com/ru-ru/windows/desktop/api/sysinfoapi/ns-sysinfoapi-_system_info#members
            winnt::PROCESSOR_ARCHITECTURE_AMD64 => Arch::X86_64,
            winnt::PROCESSOR_ARCHITECTURE_ARM => Arch::ARM,
            winnt::PROCESSOR_ARCHITECTURE_ARM64 => Arch::AARCH64,
            // TODO: Is it okay to match Ia64 to unknown arch?
            // `platforms::Arch` enum does not have specific member for Itanium.
            winnt::PROCESSOR_ARCHITECTURE_IA64 => Arch::Unknown,
            winnt::PROCESSOR_ARCHITECTURE_INTEL => Arch::X86,
            _ => Arch::Unknown,
        }
    }
}

impl fmt::Debug for Platform {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Platform")
            .field("system", &self.system())
            .field("release", &self.release())
            .field("version", &self.version())
            .field("hostname", &self.hostname())
            .field("architecture", &self.architecture())
            .finish()
    }
}

fn get_native_system_info() -> SystemInfo {
    let mut info = mem::MaybeUninit::<sysinfoapi::SYSTEM_INFO>::uninit();
    unsafe {
        // https://docs.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getnativesysteminfo
        // Returns nothing and can't fail, apparently
        sysinfoapi::GetNativeSystemInfo(info.as_mut_ptr());
        info.assume_init().into()
    }
}

/// Based on the `platform-info` crate source:
/// https://github.com/uutils/platform-info/blob/8fa071f764d55bd8e41a96cf42009da9ae20a650/src/windows.rs
fn rtl_get_version() -> winnt::OSVERSIONINFOEXW {
    let mut osinfo = mem::MaybeUninit::<winnt::RTL_OSVERSIONINFOEXW>::uninit();

    unsafe {
        (*osinfo.as_mut_ptr()).dwOSVersionInfoSize =
            mem::size_of::<winnt::RTL_OSVERSIONINFOEXW>() as minwindef::DWORD;

        let result = ntrtl::RtlGetVersion(osinfo.as_mut_ptr() as *mut _);

        // Should work all the time.
        // https://docs.microsoft.com/en-us/windows/desktop/devnotes/rtlgetversion#return-value
        debug_assert!(result == ntstatus::STATUS_SUCCESS);

        osinfo.assume_init()
    }
}

fn get_computer_name() -> Result<String> {
    let mut buffer: Vec<winnt::WCHAR> = Vec::with_capacity((MAX_COMPUTERNAME_LENGTH + 1) as usize);
    let mut size: minwindef::DWORD = MAX_COMPUTERNAME_LENGTH + 1;

    let result = unsafe { winbase::GetComputerNameW(buffer.as_mut_ptr(), &mut size) };
    if result == 0 {
        Err(Error::last_os_error().with_ffi("GetComputerNameW"))
    } else {
        unsafe {
            buffer.set_len(size as usize + 1);
        }
        let str = OsString::from_wide(&buffer[..(size as usize)])
            .to_string_lossy()
            .to_string();
        Ok(str)
    }
}

pub async fn platform() -> Result<Platform> {
    let version = rtl_get_version();

    Ok(Platform {
        sysinfo: get_native_system_info(),
        version,
        hostname: get_computer_name()?,
        build: format!("{}", version.dwBuildNumber),
    })
}
