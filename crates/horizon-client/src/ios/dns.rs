//! SRV through iOS's mDNSResponder (dns_sd), which follows the active network, VPNs and Private
//! Relay as Android's resolver does. horizon-protocol decodes a complete DNS message, as Android's
//! `android_res_nresult` returns, so the record dns_sd delivers is wrapped in one.
use std::{
    ffi::{CString, c_char, c_int, c_void},
    time::{Duration, Instant},
};

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

/// Deliver "no such record" too, instead of waiting out the timeout for every server without one.
const RETURN_INTERMEDIATES: u32 = 0x1000;
const FLAGS_ADD: u32 = 0x2;
const NO_SUCH_NAME: i32 = -65538;
const NO_SUCH_RECORD: i32 = -65554;
const TYPE_SRV: u16 = 33;
const CLASS_IN: u16 = 1;

type DnsServiceRef = *mut c_void;
type Callback = unsafe extern "C" fn(
    sd_ref: DnsServiceRef,
    flags: u32,
    interface_index: u32,
    error_code: i32,
    fullname: *const c_char,
    rrtype: u16,
    rrclass: u16,
    rdlen: u16,
    rdata: *const u8,
    ttl: u32,
    context: *mut c_void,
);

#[link(name = "System", kind = "dylib")]
unsafe extern "C" {
    fn DNSServiceQueryRecord(
        sd_ref: *mut DnsServiceRef,
        flags: u32,
        interface_index: u32,
        fullname: *const c_char,
        rrtype: u16,
        rrclass: u16,
        callback: Callback,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRefSockFD(sd_ref: DnsServiceRef) -> c_int;
    fn DNSServiceProcessResult(sd_ref: DnsServiceRef) -> i32;
    fn DNSServiceRefDeallocate(sd_ref: DnsServiceRef);
    fn poll(fds: *mut PollFd, nfds: u32, timeout: c_int) -> c_int;
}

enum Answer {
    Pending,
    Record { ttl: u32, data: Vec<u8> },
    Missing,
}

unsafe extern "C" fn reply(
    _sd_ref: DnsServiceRef,
    flags: u32,
    _interface_index: u32,
    error_code: i32,
    _fullname: *const c_char,
    rrtype: u16,
    _rrclass: u16,
    rdlen: u16,
    rdata: *const u8,
    ttl: u32,
    context: *mut c_void,
) {
    // SAFETY: `context` is query_srv's `answer`, alive until the query is deallocated; this
    // callback runs only inside DNSServiceProcessResult, on that thread.
    let answer = unsafe { &mut *context.cast::<Answer>() };
    if !matches!(answer, Answer::Pending) {
        return;
    }
    if error_code == NO_SUCH_NAME || error_code == NO_SUCH_RECORD {
        *answer = Answer::Missing;
    } else if error_code == 0
        && flags & FLAGS_ADD != 0
        && rrtype == TYPE_SRV
        && rdlen >= 7
        && !rdata.is_null()
    {
        // SAFETY: dns_sd hands `rdlen` bytes of record data for the length of the callback.
        let data = unsafe { std::slice::from_raw_parts(rdata, usize::from(rdlen)) }.to_vec();
        *answer = Answer::Record { ttl, data };
    }
}

/// Complete DNS message, parsed and checked by horizon-protocol's SRV decoder.
pub fn query_srv(name: &str) -> Option<Vec<u8>> {
    let question = encode_name(name)?;
    let fullname = CString::new(name).ok()?;
    let mut answer = Answer::Pending;
    let mut sd_ref: DnsServiceRef = std::ptr::null_mut();
    // SAFETY: `answer` outlives the query, which is deallocated before this returns; every
    // pointer passed is live for the call.
    unsafe {
        let error = DNSServiceQueryRecord(
            &mut sd_ref,
            RETURN_INTERMEDIATES,
            0,
            fullname.as_ptr(),
            TYPE_SRV,
            CLASS_IN,
            reply,
            (&raw mut answer).cast(),
        );
        if error != 0 || sd_ref.is_null() {
            return None;
        }
        let fd = DNSServiceRefSockFD(sd_ref);
        let deadline = Instant::now() + Duration::from_secs(5);
        while fd >= 0 && matches!(answer, Answer::Pending) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut ready = PollFd {
                fd,
                events: 1,
                revents: 0,
            };
            if poll(&mut ready, 1, left.as_millis() as c_int) <= 0 || ready.revents & 1 == 0 {
                break;
            }
            if DNSServiceProcessResult(sd_ref) != 0 {
                break;
            }
        }
        DNSServiceRefDeallocate(sd_ref);
    }
    match answer {
        Answer::Record { ttl, data } => Some(message(&question, Some((ttl, &data)))),
        Answer::Missing => Some(message(&question, None)),
        Answer::Pending => None,
    }
}

/// `name` in DNS wire format: length-prefixed labels and the root's empty one.
fn encode_name(name: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(name.len() + 2);
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    (out.len() <= 255).then_some(out)
}

/// A response to the one SRV question: its record, or NXDOMAIN when there is none.
fn message(question: &[u8], record: Option<(u32, &[u8])>) -> Vec<u8> {
    let found = record.is_some();
    let mut msg = Vec::with_capacity(32 + question.len() + record.map_or(0, |r| r.1.len()));
    // ID, flags (response, recursion desired and available, NOERROR or NXDOMAIN), one question,
    // and one answer or none.
    msg.extend_from_slice(&[0, 0, 0x81, if found { 0x80 } else { 0x83 }]);
    msg.extend_from_slice(&[0, 1, 0, u8::from(found), 0, 0, 0, 0]);
    msg.extend_from_slice(question);
    msg.extend_from_slice(&TYPE_SRV.to_be_bytes());
    msg.extend_from_slice(&CLASS_IN.to_be_bytes());
    if let Some((ttl, data)) = record {
        // The owner name points back at the question's, at offset 12.
        msg.extend_from_slice(&[0xc0, 12]);
        msg.extend_from_slice(&TYPE_SRV.to_be_bytes());
        msg.extend_from_slice(&CLASS_IN.to_be_bytes());
        msg.extend_from_slice(&ttl.to_be_bytes());
        msg.extend_from_slice(&(data.len() as u16).to_be_bytes());
        msg.extend_from_slice(data);
    }
    msg
}
