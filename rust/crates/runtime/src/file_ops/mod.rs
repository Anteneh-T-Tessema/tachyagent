//! File system operations: read, write, edit, glob, grep, and directory listing.

mod directory;
mod read_write;
mod search;

pub use directory::{DirEntry, ListDirectoryOutput, list_directory};
pub use read_write::{
    DiffPreview, EditFileOutput, ReadFileOutput, StructuredPatchHunk, TextFilePayload,
    WriteFileOutput, edit_file, preview_edit_file, preview_write_file, read_file, write_file,
};
pub use search::{GlobSearchOutput, GrepSearchInput, GrepSearchOutput, glob_search, grep_search};

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{edit_file, glob_search, grep_search, list_directory, read_file, write_file, GrepSearchInput};

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("tachy-native-{name}-{unique}"))
    }

    #[test]
    fn reads_and_writes_files() {
        let path = temp_path("read-write.txt");
        let (write_output, diff_preview) = write_file(path.to_string_lossy().as_ref(), "one\ntwo\nthree")
            .expect("write should succeed");
        assert_eq!(write_output.kind, "create");
        assert!(diff_preview.is_new_file);
        assert!(diff_preview.additions > 0);

        let read_output = read_file(path.to_string_lossy().as_ref(), Some(1), Some(1))
            .expect("read should succeed");
        assert_eq!(read_output.file.content, "two");
    }

    #[test]
    fn edits_file_contents() {
        let path = temp_path("edit.txt");
        write_file(path.to_string_lossy().as_ref(), "alpha beta alpha")
            .expect("initial write should succeed");
        let (output, diff_preview) = edit_file(path.to_string_lossy().as_ref(), "alpha", "omega", true)
            .expect("edit should succeed");
        assert!(output.replace_all);
        assert!(diff_preview.additions > 0 || diff_preview.deletions > 0);
        assert!(!diff_preview.diff_text.is_empty());
    }

    #[test]
    fn edit_file_fuzzy_matches_whitespace() {
        let path = temp_path("fuzzy-edit.txt");
        write_file(
            path.to_string_lossy().as_ref(),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .expect("write should succeed");

        let (output, _preview) = edit_file(
            path.to_string_lossy().as_ref(),
            "\tprintln!(\"hello\");",
            "    println!(\"world\");",
            false,
        )
        .expect("fuzzy edit should succeed");
        assert!(!output.old_string.contains('\t'));

        let content = std::fs::read_to_string(&path).expect("read back");
        assert!(content.contains("world"));
        assert!(!content.contains("hello"));
    }

    #[test]
    fn edit_file_fuzzy_matches_trailing_whitespace() {
        let path = temp_path("trailing-edit.txt");
        write_file(
            path.to_string_lossy().as_ref(),
            "line one\nline two\nline three\n",
        )
        .expect("write should succeed");

        let (output, _preview) = edit_file(
            path.to_string_lossy().as_ref(),
            "line one  \nline two  ",
            "LINE ONE\nLINE TWO",
            false,
        )
        .expect("fuzzy edit should succeed");
        assert!(output.old_string.contains("line one"));
    }

    #[test]
    fn edit_file_gives_helpful_error() {
        let path = temp_path("helpful-error.txt");
        write_file(
            path.to_string_lossy().as_ref(),
            "fn main() {\n    println!(\"hello\");\n}\n",
        )
        .expect("write should succeed");

        let err = edit_file(
            path.to_string_lossy().as_ref(),
            "completely_wrong_content",
            "replacement",
            false,
        )
        .expect_err("should fail");
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn globs_and_greps_directory() {
        let dir = temp_path("search-dir");
        std::fs::create_dir_all(&dir).expect("directory should be created");
        let file = dir.join("demo.rs");
        write_file(
            file.to_string_lossy().as_ref(),
            "fn main() {\n println!(\"hello\");\n}\n",
        )
        .expect("file write should succeed");

        let globbed = glob_search("**/*.rs", Some(dir.to_string_lossy().as_ref()))
            .expect("glob should succeed");
        assert_eq!(globbed.num_files, 1);

        let grep_output = grep_search(&GrepSearchInput {
            pattern: String::from("hello"),
            path: Some(dir.to_string_lossy().into_owned()),
            glob: Some(String::from("**/*.rs")),
            output_mode: Some(String::from("content")),
            before: None,
            after: None,
            context_short: None,
            context: None,
            line_numbers: Some(true),
            case_insensitive: Some(false),
            file_type: None,
            head_limit: Some(10),
            offset: Some(0),
            multiline: Some(false),
        })
        .expect("grep should succeed");
        assert!(grep_output.content.unwrap_or_default().contains("hello"));
    }

    #[test]
    fn lists_directory_contents() {
        let dir = temp_path("list-dir");
        std::fs::create_dir_all(dir.join("subdir")).expect("subdir should be created");
        write_file(
            dir.join("file.txt").to_string_lossy().as_ref(),
            "hello",
        )
        .expect("file write should succeed");

        let output = list_directory(Some(dir.to_string_lossy().as_ref()), Some(1))
            .expect("list should succeed");
        assert_eq!(output.total, 2);
        assert!(output.entries.iter().any(|e| e.name == "file.txt" && !e.is_dir));
        assert!(output.entries.iter().any(|e| e.name == "subdir" && e.is_dir));
        assert!(!output.truncated);
    }
}
