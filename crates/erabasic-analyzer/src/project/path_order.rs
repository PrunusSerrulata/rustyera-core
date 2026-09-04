/// Compares file paths in the order used by Emuera's recursive file loader.
///
/// Emuera sorts the files in the current directory first, then visits sorted
/// child directories recursively. A flat lexical path sort does not preserve
/// that order because it can place a child directory before a file in its
/// parent directory.
#[must_use]
pub fn compare_reference_file_paths(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.replace('\\', "/");
    let right = right.replace('\\', "/");
    let left = left
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let right = right
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let (Some((left_file, left_dirs)), Some((right_file, right_dirs))) =
        (left.split_last(), right.split_last())
    else {
        return left.cmp(&right);
    };

    for (left_dir, right_dir) in left_dirs.iter().zip(right_dirs) {
        match left_dir.cmp(right_dir) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    match left_dirs.len().cmp(&right_dirs.len()) {
        std::cmp::Ordering::Equal => left_file.cmp(right_file),
        ordering => ordering,
    }
}
