// SPDX-License-Identifier: AGPL-3.0-only
use crate::{MAX_INPUT_BYTES, Result};
use std::{fs::File, io::Read, path::Path};

pub(crate) fn chart_name(name: &str) -> Result<String> {
    let base = name.strip_suffix(".c2s").unwrap_or(name);
    if base.is_empty()
        || base.len() > 80
        || !base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("Invalid chart filename".into());
    }
    // Windows device names remain reserved even with a .c2s extension.
    let upper = base.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit())
    {
        return Err("Invalid chart filename".into());
    }
    Ok(format!("{base}.c2s"))
}

pub(crate) enum ReadError {
    NotFound,
    TooLarge,
    Invalid,
}

pub(crate) struct ChartRoot {
    directory: File,
    #[cfg(windows)]
    path: std::path::PathBuf,
}
impl ChartRoot {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .map_err(|_| "Chart directory does not exist")?;
        #[cfg(unix)]
        let directory = File::open(&path);
        #[cfg(windows)]
        let directory = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
            };
            // Keep the configured root open without delete sharing.
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&path)
        };
        let directory = directory.map_err(|_| "Cannot open chart directory")?;
        if !directory
            .metadata()
            .map_err(|_| "Cannot inspect chart directory")?
            .is_dir()
        {
            return Err("chart_dir must be a directory".into());
        }
        Ok(Self {
            directory,
            #[cfg(windows)]
            path,
        })
    }

    pub(crate) fn read(&self, name: &str) -> std::result::Result<String, ReadError> {
        let name = chart_name(name).map_err(|_| ReadError::NotFound)?;
        #[cfg(unix)]
        let file: File = {
            use rustix::fs::{Mode, OFlags, openat};
            // One validated path component, opened relative to the retained
            // directory handle. No check/open race and no symlink following.
            openat(
                &self.directory,
                name.as_str(),
                OFlags::RDONLY
                    | OFlags::CLOEXEC
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::NOCTTY,
                Mode::empty(),
            )
            .map_err(|_| ReadError::NotFound)?
            .into()
        };
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
            };
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(&name))
                .map_err(|_| ReadError::NotFound)?;
            if file
                .metadata()
                .map_err(|_| ReadError::NotFound)?
                .file_attributes()
                & FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                return Err(ReadError::NotFound);
            }
            // Check the opened handle, never a path that could be swapped after
            // canonicalization. Also rejects junction escapes through parents.
            if final_path(&file)?.parent() != Some(final_path(&self.directory)?.as_path()) {
                return Err(ReadError::NotFound);
            }
            file
        };
        read_open_file(file)
    }
}

fn read_open_file(file: File) -> std::result::Result<String, ReadError> {
    let meta = file.metadata().map_err(|_| ReadError::Invalid)?;
    if !meta.is_file() {
        return Err(ReadError::NotFound);
    }
    if meta.len() > MAX_INPUT_BYTES as u64 {
        return Err(ReadError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadError::Invalid)?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ReadError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| ReadError::Invalid)
}

#[cfg(windows)]
fn final_path(file: &File) -> std::result::Result<std::path::PathBuf, ReadError> {
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buffer = vec![0u16; 512];
    loop {
        // SAFETY: the file owns a live handle, and buffer is writable for its
        // entire length. The API reports the required size on truncation.
        let size = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                0,
            )
        } as usize;
        if size == 0 || size > 32768 {
            return Err(ReadError::NotFound);
        }
        if size < buffer.len() {
            return Ok(std::ffi::OsString::from_wide(&buffer[..size]).into());
        }
        buffer.resize(size + 1, 0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::symlink,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[test]
    fn retained_root_handle_survives_path_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("charts");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("demo.c2s"), "original").unwrap();
        let store = ChartRoot::open(&path).unwrap();
        fs::rename(&path, temp.path().join("moved")).unwrap();
        fs::create_dir(&path).unwrap();
        fs::write(path.join("demo.c2s"), "replacement").unwrap();
        assert_eq!(store.read("demo").ok().unwrap(), "original");
    }

    #[test]
    fn swapping_a_chart_for_a_symlink_never_reads_outside_data() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("charts");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("demo.c2s"), "inside").unwrap();
        let outside = temp.path().join("private.c2s");
        fs::write(&outside, "outside-canary").unwrap();
        let store = ChartRoot::open(&path).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let writer_done = done.clone();
        let writer = std::thread::spawn(move || {
            while !writer_done.load(Ordering::Relaxed) {
                symlink(&outside, path.join("swap-link")).unwrap();
                fs::rename(path.join("swap-link"), path.join("demo.c2s")).unwrap();
                fs::write(path.join("swap-regular"), "inside").unwrap();
                fs::rename(path.join("swap-regular"), path.join("demo.c2s")).unwrap();
            }
        });
        let mut escaped = false;
        for _ in 0..2000 {
            if let Ok(data) = store.read("demo") {
                escaped |= data != "inside";
            }
        }
        done.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        assert!(!escaped, "A path race escaped the retained directory");
    }
}
