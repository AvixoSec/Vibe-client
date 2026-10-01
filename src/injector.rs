use std::ffi::CString;
use std::mem;
use std::ptr;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::LibraryLoader::*;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::System::Threading::*;

fn enable_debug_privilege() -> bool {
    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            return false;
        }

        let priv_name = CString::new("SeDebugPrivilege").unwrap();
        let mut luid: LUID = mem::zeroed();
        if LookupPrivilegeValueA(ptr::null(), priv_name.as_ptr() as *const u8, &mut luid) == 0 {
            CloseHandle(token);
            return false;
        }

        let mut tp: TOKEN_PRIVILEGES = mem::zeroed();
        tp.PrivilegeCount = 1;
        tp.Privileges[0].Luid = luid;
        tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;

        let result = AdjustTokenPrivileges(
            token,
            0,
            &tp,
            mem::size_of::<TOKEN_PRIVILEGES>() as u32,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        CloseHandle(token);
        result != 0
    }
}

fn find_process(names: &[&str]) -> Option<(u32, String)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut pe: PROCESSENTRY32 = mem::zeroed();
        pe.dwSize = mem::size_of::<PROCESSENTRY32>() as u32;

        if Process32First(snap, &mut pe) == 0 {
            CloseHandle(snap);
            return None;
        }

        let mut candidates = Vec::new();

        loop {
            let exe_name_bytes: Vec<u8> = pe.szExeFile.iter()
                .take_while(|&&b| b != 0)
                .map(|&b| b as u8)
                .collect();
            let exe_name = String::from_utf8_lossy(&exe_name_bytes).to_string();
            let exe_lower = exe_name.to_lowercase();

            for &name in names {
                if exe_lower == name.to_lowercase() || exe_lower.contains(&name.to_lowercase()) {
                    candidates.push((pe.th32ProcessID, exe_name.clone()));
                    break;
                }
            }

            if Process32Next(snap, &mut pe) == 0 {
                break;
            }
        }

        CloseHandle(snap);

        // Filter out launcher processes
        let game_candidates: Vec<_> = candidates.into_iter()
            .filter(|(_, name)| !name.to_lowercase().contains("launcher"))
            .collect();

        // Prefer exact "rustme.exe" (lowercase game binary)
        if let Some(exact) = game_candidates.iter().find(|(_, name)| name == "rustme.exe") {
            return Some(exact.clone());
        }

        game_candidates.into_iter().next()
    }
}

// Raw extern for WriteProcessMemory (not always in windows-sys features)
extern "system" {
    fn WriteProcessMemory(
        hProcess: HANDLE,
        lpBaseAddress: *const std::ffi::c_void,
        lpBuffer: *const std::ffi::c_void,
        nSize: usize,
        lpNumberOfBytesWritten: *mut usize,
    ) -> BOOL;
}

fn inject_dll(pid: u32, dll_path: &str) -> Result<(), String> {
    unsafe {
        let process = OpenProcess(
            PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ,
            0,
            pid,
        );
        if process.is_null() {
            let err = GetLastError();
            return Err(format!("OpenProcess failed: error {} (run as Administrator?)", err));
        }

        let dll_cstr = CString::new(dll_path).map_err(|e| format!("Invalid DLL path: {}", e))?;
        let dll_bytes = dll_cstr.as_bytes_with_nul();
        let alloc_size = dll_bytes.len();

        let remote_mem = VirtualAllocEx(
            process,
            ptr::null(),
            alloc_size,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote_mem.is_null() {
            CloseHandle(process);
            return Err(format!("VirtualAllocEx failed"));
        }

        let mut bytes_written: usize = 0;
        if WriteProcessMemory(
            process,
            remote_mem,
            dll_bytes.as_ptr() as *const _,
            alloc_size,
            &mut bytes_written,
        ) == 0 {
            VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process);
            return Err(format!("WriteProcessMemory failed"));
        }

        let kernel32 = CString::new("kernel32.dll").unwrap();
        let load_lib = CString::new("LoadLibraryA").unwrap();
        let k32_handle = GetModuleHandleA(kernel32.as_ptr() as *const u8);
        if k32_handle == 0isize as _ {
            VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process);
            return Err("Failed to get kernel32.dll handle".to_string());
        }

        let load_lib_addr = GetProcAddress(k32_handle, load_lib.as_ptr() as *const u8);
        if load_lib_addr.is_none() {
            VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process);
            return Err("Failed to get LoadLibraryA address".to_string());
        }

        let thread_proc: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32 =
            mem::transmute(load_lib_addr.unwrap());

        let thread = CreateRemoteThread(
            process,
            ptr::null(),
            0,
            Some(thread_proc),
            remote_mem,
            0,
            ptr::null_mut(),
        );
        if thread.is_null() {
            VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
            CloseHandle(process);
            return Err(format!("CreateRemoteThread failed"));
        }

        // Wait up to 10 seconds
        let wait_result = WaitForSingleObject(thread, 10000);
        if wait_result != 0 {
            println!("  [!] Warning: injection thread did not complete within timeout");
        }

        // Check if LoadLibrary returned non-null (success)
        let mut exit_code: u32 = 0;
        GetExitCodeThread(thread, &mut exit_code);
        let success = exit_code != 0;

        VirtualFreeEx(process, remote_mem, 0, MEM_RELEASE);
        CloseHandle(thread);
        CloseHandle(process);

        if success {
            Ok(())
        } else {
            Err("LoadLibraryA returned NULL — DLL failed to load in target process".to_string())
        }
    }
}

fn main() {
    println!("\x1b[36m╔══════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[36m║  \x1b[1;37mVIBE CLIENT INJECTOR v0.1.0\x1b[0m\x1b[36m                 ║\x1b[0m");
    println!("\x1b[36m║  \x1b[90mTarget: RustMe / javaw.exe / java.exe\x1b[0m\x1b[36m       ║\x1b[0m");
    println!("\x1b[36m╚══════════════════════════════════════════════╝\x1b[0m");
    println!();

    let args: Vec<String> = std::env::args().collect();
    let custom_process = args.get(1).map(|s| s.as_str());
    let custom_dll = args.get(2).map(|s| s.as_str());

    // Resolve DLL path: ensure it is always an absolute path
    let dll_path = if let Some(path) = custom_dll {
        let p = std::path::Path::new(path);
        let abs = if p.is_relative() {
            std::env::current_dir()
                .map(|cd| cd.join(p))
                .unwrap_or_else(|_| p.to_path_buf())
        } else {
            p.to_path_buf()
        };
        abs.to_string_lossy().to_string()
    } else {
        let exe_path = std::env::current_exe().unwrap_or_default();
        let exe_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
        exe_dir.join("vibe_client.dll").to_string_lossy().to_string()
    };

    if !std::path::Path::new(&dll_path).exists() {
        println!("\x1b[31m  [x] DLL not found: {}\x1b[0m", dll_path);
        println!("      Build with: cargo build --release");
        std::process::exit(1);
    }
    println!("\x1b[32m  [+]\x1b[0m DLL: \x1b[37m{}\x1b[0m", dll_path);

    // Enable debug privilege
    print!("  [*] Enabling SeDebugPrivilege... ");
    if enable_debug_privilege() {
        println!("\x1b[32mOK\x1b[0m");
    } else {
        println!("\x1b[33mFailed (may need admin)\x1b[0m");
    }

    // Find target process (PID or name search)
    let (pid, proc_name) = if let Some(arg) = custom_process {
        if let Ok(numeric_pid) = arg.parse::<u32>() {
            println!("  [+] Using specified PID: \x1b[36m{}\x1b[0m", numeric_pid);
            (numeric_pid, format!("PID {}", numeric_pid))
        } else {
            let search_names = vec![arg];
            print!("  [*] Searching for process: {:?}... ", search_names);
            match find_process(&search_names) {
                Some(result) => {
                    println!("\x1b[32mFound!\x1b[0m");
                    result
                }
                None => {
                    println!("\x1b[31mNot found\x1b[0m");
                    std::process::exit(1);
                }
            }
        }
    } else {
        let search_names = vec!["rustme.exe", "javaw.exe", "java.exe"];
        print!("  [*] Searching for game process: {:?}... ", search_names);
        match find_process(&search_names) {
            Some(result) => {
                println!("\x1b[32mFound!\x1b[0m");
                result
            }
            None => {
                println!("\x1b[31mNot found\x1b[0m");
                println!();
                println!("\x1b[33m  Launch RustMe first, then run the injector.\x1b[0m");
                println!("  Usage: vibe_injector [PID or process_name] [dll_path]");
                std::process::exit(1);
            }
        }
    };

    println!("\x1b[32m  [+]\x1b[0m Target: \x1b[1;37m{}\x1b[0m (PID: \x1b[36m{}\x1b[0m)", proc_name, pid);

    // Inject
    print!("  [*] Injecting DLL... ");
    match inject_dll(pid, &dll_path) {
        Ok(()) => {
            println!("\x1b[32mSuccess!\x1b[0m");
            println!();
            println!("\x1b[36m  ===============================================\x1b[0m");
            println!("\x1b[1;32m  Vibe Client injected successfully.\x1b[0m");
            println!("\x1b[36m  ===============================================\x1b[0m");
        }
        Err(e) => {
            println!("\x1b[31mFailed\x1b[0m");
            println!("\x1b[31m  [x] {}\x1b[0m", e);
            std::process::exit(1);
        }
    }
}
