//! SRV through iOS mDNSResponder (dns_sd), which handles VPN and Private Relay.
use std::ffi::{CString, c_char, c_int, c_void};
use std::time::Duration;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

const DNS_SERVICE_FLAGS_ADD: u32 = 0x2;
const K_DNS_SERVICE_TYPE_SRV: u16 = 33;
const K_DNS_SERVICE_CLASS_IN: u16 = 1;

type DNSServiceRef = *mut c_void;
type DNSServiceErrorType = i32;

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn DNSServiceQueryRecord(
        sd_ref: *mut DNSServiceRef,
        flags: u32,
        interface_index: u32,
        fullname: *const c_char,
        rrtype: u16,
        rrclass: u16,
        callback: unsafe extern "C" fn(
            sd_ref: DNSServiceRef,
            flags: u32,
            interface_index: u32,
            error_code: DNSServiceErrorType,
            fullname: *const c_char,
            rrtype: u16,
            rrclass: u16,
            rdlen: u16,
            rdata: *const u8,
            ttl: u32,
            context: *mut c_void,
        ),
        context: *mut c_void,
    ) -> DNSServiceErrorType;
    fn DNSServiceRefSockFD(sd_ref: DNSServiceRef) -> c_int;
    fn DNSServiceProcessResult(sd_ref: DNSServiceRef) -> DNSServiceErrorType;
    fn DNSServiceRefDeallocate(sd_ref: DNSServiceRef);
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
}

struct QueryResult {
    data: Option<Vec<u8>>,
}

unsafe extern "C" fn query_callback(
    _sd_ref: DNSServiceRef,
    _flags: u32,
    _interface_index: u32,
    error_code: DNSServiceErrorType,
    _fullname: *const c_char,
    _rrtype: u16,
    _rrclass: u16,
    rdlen: u16,
    rdata: *const u8,
    _ttl: u32,
    context: *mut c_void,
) {
    let result = unsafe { &mut *(context as *mut QueryResult) };
    if error_code == 0 && rdlen > 0 && !rdata.is_null() {
        let data = unsafe { std::slice::from_raw_parts(rdata, rdlen as usize) };
        result.data = Some(data.to_vec());
    }
}

pub fn query_srv(name: &str) -> Option<Vec<u8>> {
    let name = CString::new(name).ok()?;
    let mut sd_ref: DNSServiceRef = std::ptr::null_mut();
    let mut result = QueryResult { data: None };

    unsafe {
        let err = DNSServiceQueryRecord(
            &mut sd_ref,
            0,
            0,
            name.as_ptr(),
            K_DNS_SERVICE_TYPE_SRV,
            K_DNS_SERVICE_CLASS_IN,
            query_callback,
            &mut result as *mut QueryResult as *mut c_void,
        );
        if err != 0 || sd_ref.is_null() {
            return None;
        }

        let fd = DNSServiceRefSockFD(sd_ref);
        if fd < 0 {
            DNSServiceRefDeallocate(sd_ref);
            return None;
        }

        let mut ready = PollFd {
            fd,
            events: 1,
            revents: 0,
        };
        let timeout_ms = 5000;
        if poll(&mut ready, 1, timeout_ms) <= 0 || ready.revents & 1 == 0 {
            DNSServiceRefDeallocate(sd_ref);
            return None;
        }

        let _ = DNSServiceProcessResult(sd_ref);
        DNSServiceRefDeallocate(sd_ref);
    }

    result.data
}
