//! Windows: a protected DACL with one entry, full control for the user of
//! the process token (night decision D42). The only `unsafe` code of the
//! crate: advapi32 calls on buffers this module allocates and handles it
//! owns or borrows from an open `File`.
#![allow(unsafe_code)]

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    GetSecurityInfo, SE_FILE_OBJECT, SetSecurityInfo,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, ACL_SIZE_INFORMATION, AclSizeInformation,
    AddAccessAllowedAce, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
    GetLengthSid, GetSecurityDescriptorControl, GetTokenInformation, InitializeAcl,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_WRITE, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The handle needs WRITE_DAC to set the DACL.
pub fn prepare(options: &mut OpenOptions) {
    options.access_mode(FILE_GENERIC_WRITE | WRITE_DAC | READ_CONTROL);
}

/// A process token, closed on drop.
struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcessToken and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// The `TOKEN_USER` of the process, in a buffer aligned for it.
struct TokenUserBuffer(Vec<u64>);

impl TokenUserBuffer {
    fn current() -> io::Result<Self> {
        let mut handle: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo handle that needs no
        // closing; `handle` receives a new token handle owned by `Token`.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Token(handle);
        let mut len = 0u32;
        // SAFETY: a null buffer of length 0 only asks for the size.
        unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut len) };
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
        // SAFETY: the buffer holds at least `len` bytes, aligned to 8 (more
        // than TOKEN_USER needs).
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                len,
                &mut len,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buffer))
    }

    /// The user's SID; it points into the buffer, so it lives as long as
    /// `self`.
    fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation(TokenUser) wrote a TOKEN_USER at the
        // start of the buffer, its Sid pointing into the same buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

pub fn restrict(file: &File) -> io::Result<()> {
    let user = TokenUserBuffer::current()?;
    let sid = user.sid();
    // SAFETY: `sid` is the valid SID of the token (above).
    let sid_len = unsafe { GetLengthSid(sid) } as usize;
    let acl_len = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_len;
    let mut acl = vec![0u64; acl_len.div_ceil(8)];
    let acl_ptr = acl.as_mut_ptr().cast::<ACL>();
    let acl_len = u32::try_from(acl_len).map_err(|_| io::Error::other("ACL too large"))?;
    // SAFETY: the buffer holds `acl_len` bytes, aligned to 8 (ACL needs 4).
    if unsafe { InitializeAcl(acl_ptr, acl_len, ACL_REVISION) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the ACL was initialized with room for this one entry.
    if unsafe { AddAccessAllowedAce(acl_ptr, ACL_REVISION, FILE_ALL_ACCESS, sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is the open file's, opened with WRITE_DAC; the
    // owner, group and SACL are not changed (null).
    let status = unsafe {
        SetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            acl_ptr,
            ptr::null(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(())
}

/// A security descriptor from GetSecurityInfo, freed on drop.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated it with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

pub fn is_private(path: &Path) -> io::Result<bool> {
    let file = File::open(path)?;
    let user = TokenUserBuffer::current()?;
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: the handle is the open file's (GENERIC_READ includes
    // READ_CONTROL); `dacl` points into `descriptor`, which is freed last.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle() as HANDLE,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = Descriptor(descriptor);
    if dacl.is_null() {
        // No DACL at all: everyone has access.
        return Ok(false);
    }
    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: the descriptor is valid until `descriptor` drops.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut info = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: `info` is an ACL_SIZE_INFORMATION of the size passed.
    let ok = unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 || info.AceCount != 1 {
        return Ok(false);
    }
    let mut ace: *mut core::ffi::c_void = ptr::null_mut();
    // SAFETY: index 0 exists (AceCount is 1).
    if unsafe { GetAce(dacl, 0, &mut ace) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let ace = ace.cast::<ACCESS_ALLOWED_ACE>();
    // SAFETY: GetAce returned a pointer to the entry inside the DACL; an
    // ACCESS_ALLOWED_ACE keeps its SID at `SidStart`.
    let allowed = unsafe {
        u32::from((*ace).Header.AceType) == ACCESS_ALLOWED_ACE_TYPE
            && EqualSid(ptr::addr_of!((*ace).SidStart).cast_mut().cast(), user.sid()) != 0
    };
    drop(descriptor);
    Ok(allowed)
}
