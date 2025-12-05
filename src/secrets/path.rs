use std::path::{Component, Path, PathBuf};
extern crate sanitize_filename;
pub trait PathExt {
    fn clean(&self) -> PathBuf;
    fn absolute(&self) -> PathBuf;
}

impl PathExt for Path
{
    fn clean(&self) -> PathBuf {
        let mut components = self.components().peekable();
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
    fn absolute(&self) -> PathBuf {
        let anchored = std::path::absolute(self).unwrap_or_else(|_| {
            self.to_path_buf()
        });
        anchored.clean()
    }
}
