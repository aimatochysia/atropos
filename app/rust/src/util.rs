use std::path::Path;

pub(crate) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string()
}
