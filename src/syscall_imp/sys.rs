pub(crate) struct Utsname {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domainname: [u8; 65],
}
pub fn sys_uname(buf: *mut Utsname) -> i32 {
    let utsname = Utsname {
        sysname: {
            let mut arr = [0u8; 65];
            arr[..7].copy_from_slice(b"ArceOS\0");
            arr
        },
        nodename: {
            let mut arr = [0u8; 65];
            arr[..7].copy_from_slice(b"ArceOS\0");
            arr
        },
        release: {
            let mut arr = [0u8; 65];
            arr[..6].copy_from_slice(b"0.1.0\0");
            arr
        },
        version: {
            let mut arr = [0u8; 65];
            arr[..6].copy_from_slice(b"0.1.0\0");
            arr
        },
        machine: {
            let mut arr = [0u8; 65];
            arr[..8].copy_from_slice(b"riscv64\0");
            arr
        },
        domainname: {
            let mut arr = [0u8; 65];
            arr[..1].copy_from_slice(b"\0");
            arr
        },
    };
    unsafe {
        buf.write(utsname);
    }
    0
}
