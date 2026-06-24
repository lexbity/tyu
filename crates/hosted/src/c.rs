#![allow(non_camel_case_types)]

use core::ffi::c_void;

pub type c_char = i8;
pub type c_int = i32;
pub type size_t = usize;
pub type ssize_t = isize;

pub type FILE = c_void;

#[link(name = "c")]
extern "C" {
    pub fn __errno_location() -> *mut c_int;

    pub fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;

    pub fn getenv(name: *const c_char) -> *const c_char;

    pub fn malloc(size: size_t) -> *mut c_void;
    pub fn realloc(ptr: *mut c_void, size: size_t) -> *mut c_void;
    pub fn free(ptr: *mut c_void);
    pub fn _exit(status: c_int) -> !;

    pub fn fopen(path: *const c_char, mode: *const c_char) -> *mut FILE;
    pub fn fread(ptr: *mut c_void, size: size_t, nmemb: size_t, stream: *mut FILE) -> size_t;
    pub fn fwrite(ptr: *const c_void, size: size_t, nmemb: size_t, stream: *mut FILE) -> size_t;
    pub fn ferror(stream: *mut FILE) -> c_int;
    pub fn feof(stream: *mut FILE) -> c_int;
    pub fn fclose(stream: *mut FILE) -> c_int;

    pub fn fork() -> c_int;
    pub fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int;
    pub fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;

    // Memory mapping (S2 Phase 7)
    pub fn mmap(
        addr: *mut c_void,
        length: size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: isize,
    ) -> *mut c_void;
    pub fn mprotect(addr: *mut c_void, len: size_t, prot: c_int) -> c_int;
    pub fn munmap(addr: *mut c_void, length: size_t) -> c_int;
}
