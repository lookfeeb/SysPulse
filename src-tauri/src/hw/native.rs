//! Private .NET runtime hosting. No subprocess, pipe, request ID or JSON.
//! CLR and hostfxr stay loaded until process exit; unloading with live managed
//! threads is unsafe. Native calls are only made by the client's worker.
use std::{ffi::c_void, path::Path, ptr};
type Call = unsafe extern "C" fn(i32, i32, *const u8, i32, f64, *mut u8, i32, *mut i32) -> i32;
const MAX_FRAME: usize = 4 * 1024 * 1024;
pub struct NativeHardware {
    call: Call,
    buffer: Vec<u8>,
}

impl NativeHardware {
    pub fn load(directory: &Path) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[repr(C)]
            struct Parameters {
                size: usize,
                host_path: *const u16,
                dotnet_root: *const u16,
            }
            type Init =
                unsafe extern "C" fn(*const u16, *const Parameters, *mut *mut c_void) -> i32;
            type GetDelegate = unsafe extern "C" fn(*mut c_void, i32, *mut *mut c_void) -> i32;
            type Close = unsafe extern "C" fn(*mut c_void) -> i32;
            type Load = unsafe extern "system" fn(
                *const u16,
                *const u16,
                *const u16,
                *const u16,
                *mut c_void,
                *mut *mut c_void,
            ) -> i32;
            #[link(name = "kernel32")]
            extern "system" {
                fn LoadLibraryExW(path: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
                fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
            }
            let dir = directory
                .canonicalize()
                .map_err(|e| format!("Hardware module directory: {e}"))?;
            let wide = |p: &Path| {
                p.as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            let root = wide(&dir);
            let config = wide(&dir.join("hw-helper.runtimeconfig.json"));
            let assembly = wide(&dir.join("hw-helper.dll"));
            let dll = wide(&dir.join("hostfxr.dll"));
            let text = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            // Absolute path + restricted dependency search; never search the CWD.
            unsafe {
                let module = LoadLibraryExW(dll.as_ptr(), ptr::null_mut(), 0x100 | 0x1000);
                if module.is_null() {
                    return Err(format!(
                        "Cannot load private hostfxr: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                let symbol = |name: &[u8]| -> Result<*mut c_void, String> {
                    let address = GetProcAddress(module, name.as_ptr());
                    if address.is_null() {
                        Err("Incomplete hostfxr library".into())
                    } else {
                        Ok(address)
                    }
                };
                let init: Init =
                    std::mem::transmute(symbol(b"hostfxr_initialize_for_runtime_config\0")?);
                let get: GetDelegate =
                    std::mem::transmute(symbol(b"hostfxr_get_runtime_delegate\0")?);
                let close: Close = std::mem::transmute(symbol(b"hostfxr_close\0")?);
                let parameters = Parameters {
                    size: std::mem::size_of::<Parameters>(),
                    host_path: ptr::null(),
                    dotnet_root: root.as_ptr(),
                };
                let mut context = ptr::null_mut();
                let result = init(config.as_ptr(), &parameters, &mut context);
                if result < 0 || context.is_null() {
                    return Err(format!(
                        "Private .NET initialization failed: 0x{result:08x}"
                    ));
                }
                let mut loader = ptr::null_mut();
                let result = get(context, 5, &mut loader); // hdt_load_assembly_and_get_function_pointer
                close(context);
                if result < 0 || loader.is_null() {
                    return Err(format!("Managed loader unavailable: 0x{result:08x}"));
                }
                let load: Load = std::mem::transmute(loader);
                let typename = text("SysPulse.HwHelper.InProcessBridge, hw-helper");
                let method = text("Call");
                let mut entry = ptr::null_mut();
                let result = load(
                    assembly.as_ptr(),
                    typename.as_ptr(),
                    method.as_ptr(),
                    usize::MAX as *const u16,
                    ptr::null_mut(),
                    &mut entry,
                ); // UNMANAGEDCALLERSONLY_METHOD
                if result < 0 || entry.is_null() {
                    return Err(format!("Hardware entry point unavailable: 0x{result:08x}"));
                }
                Ok(Self {
                    call: std::mem::transmute(entry),
                    buffer: vec![0; 65536],
                })
            }
        }
        #[cfg(not(windows))]
        {
            let _ = directory;
            Err("Hardware module requires Windows".into())
        }
    }
    pub fn invoke(&mut self, op: i32, name: &str, value: f64) -> Result<&[u8], String> {
        if name.len() > 4096 {
            return Err("INVALID_PARAM: fan identifier too long".into());
        }
        for _ in 0..3 {
            let mut written = 0;
            let result = unsafe {
                (self.call)(
                    1,
                    op,
                    name.as_ptr(),
                    name.len() as i32,
                    value,
                    self.buffer.as_mut_ptr(),
                    self.buffer.len() as i32,
                    &mut written,
                )
            };
            if written < 0 || written as usize > MAX_FRAME {
                return Err("Invalid hardware ABI frame length".into());
            }
            if result == 1 && op == 1 && written as usize > self.buffer.len() {
                self.buffer.resize(written as usize, 0);
                continue;
            }
            if written as usize > self.buffer.len() {
                return Err("Hardware ABI exceeded output buffer".into());
            }
            let data = &self.buffer[..written as usize];
            if result < 0 {
                return Err(String::from_utf8_lossy(data).into_owned());
            }
            if result != 0 {
                return Err(format!("Unexpected hardware ABI status {result}"));
            }
            return Ok(data);
        }
        Err("Hardware snapshot grew beyond bounded retries".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "loads the bundled runtime and reads real hardware; no control writes"]
    fn real_in_process_lifecycle() {
        let dir = std::env::var("HW_NATIVE_DIR").expect("set HW_NATIVE_DIR to bundle directory");
        let mut hardware = NativeHardware::load(Path::new(&dir)).unwrap();
        let mut out = [0u8; 128];
        let mut length = 0;
        assert!(
            unsafe {
                (hardware.call)(
                    99,
                    0,
                    ptr::null(),
                    0,
                    0.0,
                    out.as_mut_ptr(),
                    128,
                    &mut length,
                )
            } < 0
        );
        assert!(hardware.invoke(1, "", 0.0).is_err());
        hardware.invoke(0, "", 0.0).unwrap();
        hardware.invoke(0, "", 0.0).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(6));
        let mut timings = Vec::new();
        for _ in 0..1000 {
            let start = std::time::Instant::now();
            let frame = hardware.invoke(1, "", 0.0).unwrap();
            assert_eq!(&frame[..4], b"SPW2");
            timings.push(start.elapsed().as_nanos());
        }
        timings.sort();
        println!(
            "in-process raw ABI: p50={}ns p95={}ns",
            timings[500], timings[950]
        );
        if let Ok(file) = std::env::var("HW_NATIVE_FRAME") {
            std::fs::write(file, hardware.invoke(1, "", 0.0).unwrap()).unwrap();
        }
        assert!(hardware.invoke(999, "", 0.0).is_err());
        hardware.invoke(5, "", 0.0).unwrap();
        hardware.invoke(5, "", 0.0).unwrap();
        assert!(hardware.invoke(1, "", 0.0).is_err());
    }
}
