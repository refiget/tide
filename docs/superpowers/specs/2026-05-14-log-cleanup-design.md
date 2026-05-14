# Log Cleanup Design

Maintain a manageable number of log files in the `debug/` directory.

## Requirements

1.  Before creating a new log file, check the number of existing files in the `debug/` directory.
2.  If there are 10 or more files, delete the oldest ones until only 9 remain.
3.  The new log file will then be the 10th.
4.  Handle errors gracefully (IO errors during directory reading or file deletion should not prevent the application from starting or logging).
5.  Only target files matching the pattern `tide-*.log`.

## Approach

Implement a private helper method `cleanup_old_logs(dir: &Path)` in `DebugLog`.

### cleanup_old_logs logic:
1.  Read the directory `dir`.
2.  Filter entries:
    *   Must be a file.
    *   Name must start with `tide-` and end with `.log`.
3.  Collect valid entries into a list of `(PathBuf, SystemTime)`.
4.  If list length < 10, return.
5.  Sort the list by `SystemTime` (oldest first).
6.  Identify the number of files to delete: `count = len - 9`.
7.  Iterate and delete the first `count` files.
8.  Log errors to stderr if deletion fails (since `DebugLog` isn't ready yet).

## Integration

Call `Self::cleanup_old_logs(&dir)` inside `DebugLog::open_if_enabled()` after ensuring the directory exists but before creating the new log file.

## Testing

Add a unit test `test_log_cleanup` in `src/debug_log.rs`:
1.  Create a temporary directory.
2.  Create 12 dummy "tide-*.log" files with different modification times (using `filetime` or by sleeping/waiting between creations if needed, though `std::fs::set_modified` is better if available, or just use the natural order of creation).
3.  Call `cleanup_old_logs`.
4.  Verify that exactly 9 files remain.
5.  Verify that the "oldest" files were deleted.
