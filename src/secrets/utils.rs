use std::path::{Path, PathBuf, Component};

pub(crate) fn clean(path: impl AsRef<Path>) -> PathBuf {
    normalize(make_absolute(path))
}

pub(crate) fn normalize(path: impl AsRef<Path>) -> PathBuf {
    let mut components = path.as_ref().components().peekable();
    let mut ret = if let Some(c @ Component::Prefix(..)) = components.peek().cloned() {
        components.next();
        PathBuf::from(c.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => {
                ret.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                ret.pop();
            }
            Component::Normal(c) => {
                ret.push(c);
            }
        }
    }
    ret
}

pub(crate) fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_lowercase() || lc.is_ascii_digit() || matches!(lc, '.' | '_' | '-') {
            out.push(lc);
        } else {
            out.push('_');
        }
    }
    out
}

pub(crate) fn make_absolute(path: impl AsRef<Path>) -> PathBuf {
    let p = path.as_ref();
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("/"))
        .join(p)
    }
}
