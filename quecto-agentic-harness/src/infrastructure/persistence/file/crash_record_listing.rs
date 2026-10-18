//! Listing the crash directory (#2192): its entries read through the
//! held directory's own descriptor, a bounded number of them — every entry
//! counts, whatever its name — and whether the listing reached the
//! directory's end said, never left silent.
use super::{RecordDir, c_name};

/// The most directory entries one listing examines.
pub(super) const MAX_LISTED: usize = 4096;

/// The names a listing found, and whether it read the directory to its end
/// (not cut short by its bound or by a read error).
#[derive(Debug)]
pub(super) struct Listed {
    pub(super) names: Vec<String>,
    pub(super) complete: bool,
}

/// How a listing's `readdir` stopped: at the directory's end, or on an
/// error (the `errno` it left).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListingEnd {
    End,
    Failed(i32),
}

/// A null `readdir` with `errno` still 0 is the end; any other is an error.
pub(super) fn listing_end(errno: i32) -> ListingEnd {
    match errno {
        0 => ListingEnd::End,
        failed => ListingEnd::Failed(failed),
    }
}

/// Set `errno` to `value`: 0 before a `readdir`, so a null one can be
/// told apart from an error. Answers whether it could: where it cannot, an
/// end is taken as an error.
fn set_errno(value: i32) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // SAFETY: `__errno_location` points at this thread's `errno`.
        unsafe { *libc::__errno_location() = value };
        true
    }
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    {
        // SAFETY: `__error` points at this thread's `errno`.
        unsafe { *libc::__error() = value };
        true
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd"
    )))]
    {
        let _ = value;
        false
    }
}

/// A test's fault in a real listing (#2192 review): after `after` entries
/// are read, the next `readdir` fails as the system's would — a null entry
/// with `errno` set to `errno`.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(super) struct ReadFault {
    pub(super) after: usize,
    pub(super) errno: i32,
}

/// The names in `dir` — the held directory, not whatever is at its path
/// now — that `keep` keeps, sorted, from at most `examined` entries: a
/// directory anyone can fill is never read without end. Every entry read
/// counts, a name that is not UTF-8 too (#2192 review). When the bound is
/// reached, one more read tells a directory of exactly that many entries
/// (complete) from a larger one (cut short).
pub(super) fn names_matching_within(
    dir: &RecordDir,
    keep: &dyn Fn(&str) -> bool,
    examined: usize,
) -> Listed {
    match Listing::open(dir) {
        Ok(listing) => collect(listing, keep, examined),
        Err(_) => Listed {
            names: Vec::new(),
            complete: false,
        },
    }
}

/// [`names_matching_within`], its listing failing as `fault` says.
#[cfg(test)]
pub(super) fn names_matching_faulted(
    dir: &RecordDir,
    keep: &dyn Fn(&str) -> bool,
    examined: usize,
    fault: ReadFault,
) -> Listed {
    let mut listing = Listing::open(dir).expect("the directory opens");
    listing.fault = Some(fault);
    collect(listing, keep, examined)
}

fn collect(mut listing: Listing, keep: &dyn Fn(&str) -> bool, examined: usize) -> Listed {
    let mut names: Vec<String> = listing
        .by_ref()
        .take(examined)
        .flatten()
        .filter(|name| keep(name))
        .collect();
    names.sort();
    let complete = match listing.ended {
        Some(_) => listing.reached_end(),
        // The bound was reached with no end seen: is anything left?
        None => listing.next().is_none() && listing.reached_end(),
    };
    Listed { names, complete }
}

/// The entries of a held directory, read through its own descriptor.
struct Listing {
    stream: std::ptr::NonNull<libc::DIR>,
    /// How its reading stopped, once it has.
    ended: Option<ListingEnd>,
    #[cfg(test)]
    fault: Option<ReadFault>,
}

impl Listing {
    /// Whether the listing read the directory to its end, without error.
    fn reached_end(&self) -> bool {
        matches!(self.ended, Some(ListingEnd::End))
    }
}

impl Listing {
    /// A fresh descriptor of the held directory (its own read position, so
    /// concurrent listings do not share one), turned into a stream.
    fn open(dir: &RecordDir) -> std::io::Result<Self> {
        use std::os::unix::io::IntoRawFd;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC;
        let fd = dir.open_at(&c_name(".")?, flags, 0)?.into_raw_fd();
        // The stream owns `fd` once fdopendir succeeds.
        // SAFETY: `fd` is an open directory descriptor owned by nothing else.
        let stream = unsafe { libc::fdopendir(fd) };
        match std::ptr::NonNull::new(stream) {
            Some(stream) => Ok(Self {
                stream,
                ended: None,
                #[cfg(test)]
                fault: None,
            }),
            None => {
                let error = std::io::Error::last_os_error();
                // SAFETY: fdopendir failed, so `fd` is still ours to close.
                unsafe { libc::close(fd) };
                Err(error)
            }
        }
    }
}

impl Iterator for Listing {
    /// An entry's name; `None` for a name that is not UTF-8 (no record has
    /// one), which is still an entry read.
    type Item = Option<String>;

    /// The next entry. A read error ends the listing like its end does,
    /// but is kept apart ([`Listing::reached_end`]).
    fn next(&mut self) -> Option<Option<String>> {
        {
            let cleared = set_errno(0);
            let entry = self.read_entry();
            let Some(entry) = std::ptr::NonNull::new(entry) else {
                let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                self.ended = Some(match cleared {
                    true => listing_end(errno),
                    false => ListingEnd::Failed(errno),
                });
                return None;
            };
            // The entry lives until the next readdir on this stream.
            // SAFETY: readdir returned a valid entry with a NUL-terminated name.
            let name = unsafe { std::ffi::CStr::from_ptr(entry.as_ref().d_name.as_ptr()) };
            Some(name.to_str().ok().map(str::to_string))
        }
    }
}

impl Listing {
    /// One `readdir` of the stream — or, in a test, the fault it asked for.
    fn read_entry(&mut self) -> *mut libc::dirent {
        #[cfg(test)]
        if let Some(fault) = self.fault.as_mut() {
            match fault.after {
                0 => {
                    set_errno(fault.errno);
                    return std::ptr::null_mut();
                }
                _ => fault.after -= 1,
            }
        }
        // SAFETY: `stream` is an open stream this listing owns.
        unsafe { libc::readdir(self.stream.as_ptr()) }
    }
}

impl Drop for Listing {
    fn drop(&mut self) {
        // SAFETY: `stream` is open and owned by this listing; closed once.
        unsafe { libc::closedir(self.stream.as_ptr()) };
    }
}

#[cfg(test)]
#[path = "crash_record_listing_tests.rs"]
mod tests;
