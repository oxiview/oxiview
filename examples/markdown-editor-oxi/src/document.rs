//! UTF-8 document storage and ordered file operations for one editor window.

use oxiview_platform::WindowWaker;
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

#[derive(Clone, Copy, Default)]
pub(crate) struct FileFormat {
    bom: bool,
    crlf: bool,
}

#[derive(Default)]
pub(crate) struct Document {
    pub(crate) path: Option<PathBuf>,
    pub(crate) format: FileFormat,
}

impl Document {
    fn open(path: PathBuf) -> io::Result<(Self, String)> {
        let raw = fs::read_to_string(&path)?;
        let (bom, body) = match raw.strip_prefix('\u{feff}') {
            Some(body) => (true, body),
            None => (false, raw.as_str()),
        };
        let crlf = body.contains("\r\n")
            && !body.as_bytes().iter().enumerate().any(|(i, &byte)| {
                byte == b'\n'
                    && i.checked_sub(1).and_then(|i| body.as_bytes().get(i)) != Some(&b'\r')
            });
        let source = if crlf {
            body.replace("\r\n", "\n")
        } else if bom {
            body.to_owned()
        } else {
            raw
        };
        Ok((
            Self {
                path: Some(path),
                format: FileFormat { bom, crlf },
            },
            source,
        ))
    }

    pub(crate) fn title(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.md".into())
    }
}

pub(crate) enum FileReply {
    Opened {
        token: u64,
        result: io::Result<(Document, String)>,
    },
    Saved {
        token: u64,
        result: io::Result<()>,
    },
}

pub(crate) enum FileRequest {
    Open(PathBuf),
    Save(PathBuf, String, FileFormat),
}

type FileJob = (u64, FileRequest, WindowWaker);

/// A FIFO worker prevents a slow earlier write from replacing a later save.
pub(crate) struct Files {
    sender: Sender<FileJob>,
    replies: Receiver<FileReply>,
}

impl Files {
    pub(crate) fn new() -> io::Result<Self> {
        let (sender, jobs) = mpsc::channel::<FileJob>();
        let (completed, replies) = mpsc::channel();
        thread::Builder::new()
            .name("markdown-files".into())
            .spawn(move || {
                for (token, request, waker) in jobs {
                    let reply = match request {
                        FileRequest::Open(path) => FileReply::Opened {
                            token,
                            result: Document::open(path),
                        },
                        FileRequest::Save(path, source, format) => FileReply::Saved {
                            token,
                            result: write_atomic(&path, &source, format),
                        },
                    };
                    if completed.send(reply).is_err() {
                        break;
                    }
                    let _ = waker.wake();
                }
            })?;
        Ok(Self { sender, replies })
    }

    pub(crate) fn open(&self, token: u64, path: PathBuf, waker: &WindowWaker) -> io::Result<()> {
        self.send(token, FileRequest::Open(path), waker)
    }

    pub(crate) fn save(
        &self,
        token: u64,
        path: PathBuf,
        source: String,
        format: FileFormat,
        waker: &WindowWaker,
    ) -> io::Result<()> {
        self.send(token, FileRequest::Save(path, source, format), waker)
    }

    fn send(&self, token: u64, job: FileRequest, waker: &WindowWaker) -> io::Result<()> {
        self.sender
            .send((token, job, waker.clone()))
            .map_err(|_| io::Error::other("The document worker has stopped"))
    }

    pub(crate) fn poll(&self) -> Option<FileReply> {
        self.replies.try_recv().ok()
    }

    #[cfg(test)]
    pub(crate) fn controlled() -> (Self, Receiver<FileJob>, Sender<FileReply>) {
        let (sender, jobs) = mpsc::channel();
        let (completed, replies) = mpsc::channel();
        (Self { sender, replies }, jobs, completed)
    }
}

fn write_atomic(path: &Path, text: &str, format: FileFormat) -> io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let destination = match fs::symlink_metadata(path) {
        Ok(m) if m.is_symlink() => fs::canonicalize(path)?,
        Ok(_) => path.to_owned(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => path.to_owned(),
        Err(e) => return Err(e),
    };
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let permissions = match fs::metadata(&destination) {
        Ok(m) if m.is_file() && !m.permissions().readonly() => Some(m.permissions()),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "The destination is not a writable regular file",
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let (staging, mut file) = loop {
        let serial = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = parent.join(format!(".oxiview-md-{}-{serial}.tmp", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&name)
        {
            Ok(file) => break (name, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?
        }
        {
            let mut writer = io::BufWriter::new(&mut file);
            if format.bom {
                writer.write_all(&[0xef, 0xbb, 0xbf])?;
            }
            if format.crlf {
                for line in text.split_inclusive('\n') {
                    if let Some(body) = line.strip_suffix('\n') {
                        writer.write_all(body.strip_suffix('\r').unwrap_or(body).as_bytes())?;
                        writer.write_all(b"\r\n")?;
                    } else {
                        writer.write_all(line.as_bytes())?;
                    }
                }
            } else {
                writer.write_all(text.as_bytes())?;
            }
            writer.flush()?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_bytes_preserve_bom_crlf_and_mixed_endings() {
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/xtask/md-document-tests");
        fs::create_dir_all(&dir).expect("test directory");
        for (i, text) in ["\u{feff}# 标题\r\n👩‍💻\r\n", "a\r\nb\nc", "e\u{301}\n"]
            .iter()
            .enumerate()
        {
            let input = dir.join(format!("input-{i}.md"));
            let output = dir.join(format!("saved-{i}.md"));
            fs::write(&input, text).expect("fixture file");
            let (doc, source) = Document::open(input).expect("open");
            write_atomic(&output, &source, doc.format).expect("write");
            assert_eq!(fs::read_to_string(output).expect("read copy"), *text);
        }
        let output = dir.join("paste-crlf.md");
        let format = FileFormat {
            bom: true,
            crlf: true,
        };
        write_atomic(&output, "a\nb\r\nc", format).expect("write pasted CRLF");
        assert_eq!(
            fs::read_to_string(output).expect("read paste"),
            "\u{feff}a\r\nb\r\nc"
        );
        assert!(write_atomic(&dir, "unwritten", FileFormat::default()).is_err());
    }
}
