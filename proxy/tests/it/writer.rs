use crate::common::{
    CLIENT, FailingFile, Failure, RUN, filled_in, log_lines, log_path, pass, start, start_writer,
};
use proxy::{LogFailed, create_log_file, log_queue, run_writer};
use std::io::ErrorKind;
use std::time::Duration;
use tally_core::{Entry, LogLine};
use tokio_util::sync::CancellationToken;

// Writing lines

#[tokio::test]
async fn lines_get_seq_client_and_time_in_submit_order() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let (writer, log_failed, thread) = start_writer(file, 1024);

    assert_eq!(writer.submit(start()).await, Ok(0));
    assert_eq!(writer.submit(pass()).await, Ok(1));
    assert_eq!(writer.submit(Entry::Stop).await, Ok(2));
    drop(writer);
    thread.join().expect("the Writer doesn't panic");

    let lines = log_lines(&log_path(folder.path()));
    assert_eq!(lines.len(), 3);
    for (expected_seq, line) in lines.iter().enumerate() {
        let (seq, client, time) = filled_in(line);
        assert_eq!(seq, expected_seq as u64);
        assert_eq!(client, CLIENT);
        time.parse::<jiff::Timestamp>().expect("an RFC 3339 time");
        assert!(
            time.ends_with('Z') && !time.contains('.'),
            "UTC, whole seconds: {time}"
        );
    }
    assert!(matches!(lines[0], LogLine::Start { .. }));
    assert!(matches!(&lines[1], LogLine::Pass { keys, .. } if keys.is_empty()));
    assert!(matches!(lines[2], LogLine::Stop { .. }));
    assert!(!log_failed.is_cancelled());
}

#[tokio::test]
async fn a_line_is_in_the_file_when_its_answer_arrives() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let (writer, log_failed, thread) = start_writer(file, 1024);
    let path = log_path(folder.path());

    writer.submit(start()).await.expect("the line is on disk");
    assert_eq!(log_lines(&path).len(), 1);
    writer.submit(pass()).await.expect("the line is on disk");
    assert_eq!(log_lines(&path).len(), 2);

    // No stop line: the Writer ends once every handle is dropped, as a normal end
    drop(writer);
    thread.join().expect("the Writer doesn't panic");
    assert!(!log_failed.is_cancelled());
}

#[tokio::test]
async fn concurrent_lines_are_all_written_before_the_stop_line() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let (writer, log_failed, thread) = start_writer(file, 1024);

    writer.submit(start()).await.expect("the line is on disk");
    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let writer = writer.clone();
            tokio::spawn(async move { writer.submit(pass()).await })
        })
        .collect();
    for task in tasks {
        task.await
            .expect("the task ends")
            .expect("the line is on disk");
    }
    writer
        .submit(Entry::Stop)
        .await
        .expect("the line is on disk");
    drop(writer);
    thread.join().expect("the Writer doesn't panic");

    let lines = log_lines(&log_path(folder.path()));
    let seqs: Vec<u64> = lines.iter().map(|line| filled_in(line).0).collect();
    assert_eq!(seqs, (0..12).collect::<Vec<u64>>());
    assert!(matches!(lines.last(), Some(LogLine::Stop { .. })));
    assert!(
        !log_failed.is_cancelled(),
        "a normal end doesn't set log_failed"
    );
}

#[tokio::test]
async fn a_line_after_the_stop_line_is_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let (writer, log_failed, thread) = start_writer(file, 1024);

    assert_eq!(writer.submit(start()).await, Ok(0));
    assert_eq!(writer.submit(Entry::Stop).await, Ok(1));
    assert_eq!(writer.submit(pass()).await, Err(LogFailed));

    // "writer" is still alive, and the Writer ends all the same
    for _ in 0..200 {
        if thread.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        thread.is_finished(),
        "the Writer ends at the stop line, with a LogWriter still alive"
    );
    thread.join().expect("the Writer doesn't panic");
    assert!(!log_failed.is_cancelled(), "the stop line is a normal end");
    let lines = log_lines(&log_path(folder.path()));
    assert!(
        matches!(
            lines.as_slice(),
            [LogLine::Start { .. }, LogLine::Stop { .. }]
        ),
        "nothing after the stop line: {lines:?}"
    );
    drop(writer);
}

#[tokio::test]
async fn a_full_queue_makes_submit_wait_without_dropping_a_line() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let (writer, queue) = log_queue(1);
    let tasks: Vec<_> = (0..3)
        .map(|_| {
            let writer = writer.clone();
            tokio::spawn(async move { writer.submit(pass()).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        tasks.iter().all(|task| !task.is_finished()),
        "no Writer yet: every submit waits"
    );
    assert_eq!(
        queue.len(),
        1,
        "one line in the queue, the other two wait for room"
    );

    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let thread = std::thread::spawn(move || {
        run_writer(file, queue, CLIENT.to_string(), CancellationToken::new())
    });
    let mut seqs = Vec::new();
    for task in tasks {
        seqs.push(
            task.await
                .expect("the task ends")
                .expect("the line is on disk"),
        );
    }
    seqs.sort_unstable();
    assert_eq!(seqs, [0, 1, 2]);
    drop(writer);
    thread.join().expect("the Writer doesn't panic");
    assert_eq!(log_lines(&log_path(folder.path())).len(), 3);
}

// The run's file

#[test]
fn a_second_log_file_for_the_same_run_is_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    create_log_file(folder.path(), RUN).expect("the first is created");
    let error = create_log_file(folder.path(), RUN).expect_err("the second is refused");
    assert_eq!(error.kind(), ErrorKind::AlreadyExists);
}

#[test]
fn a_log_folder_whose_parent_is_missing_is_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let logs = folder.path().join("missing").join("logs");
    let error = create_log_file(&logs, RUN).expect_err("only one level is created");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn a_log_folder_that_is_a_file_is_refused() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let logs = folder.path().join("logs");
    std::fs::write(&logs, b"").expect("a file named logs");
    create_log_file(&logs, RUN).expect_err("a file isn't a folder");
}

#[test]
fn a_missing_log_folder_is_created() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let logs = folder.path().join("logs");
    create_log_file(&logs, RUN).expect("the folder and the file are created");
    assert!(log_path(&logs).is_file());
}

// 1. Proxy startup, exit "The log fails while the proxy runs", on the Writer's side

#[tokio::test]
async fn a_failing_line_fails_every_line_queued_behind_it() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let (writer, queue) = log_queue(1024);
    let tasks: Vec<_> = (0..5)
        .map(|_| {
            let writer = writer.clone();
            tokio::spawn(async move { writer.submit(pass()).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The five lines are queued: the third fails, the fourth and fifth are behind it
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let failing = FailingFile {
        file,
        lines_before: 2,
        failure: Failure::Append,
    };
    let log_failed = CancellationToken::new();
    let token = log_failed.clone();
    let thread = std::thread::spawn(move || run_writer(failing, queue, CLIENT.to_string(), token));
    let mut answers = Vec::new();
    for task in tasks {
        answers.push(task.await.expect("the task ends"));
    }
    let mut on_disk: Vec<u64> = answers.iter().filter_map(|answer| answer.ok()).collect();
    on_disk.sort_unstable();
    assert_eq!(on_disk, [0, 1]);
    assert_eq!(answers.iter().filter(|answer| answer.is_err()).count(), 3);

    assert_eq!(writer.submit(pass()).await, Err(LogFailed), "a later line");
    thread.join().expect("the Writer returns, without a panic");
    assert!(log_failed.is_cancelled());
    assert_eq!(log_lines(&log_path(folder.path())).len(), 2);
}

#[tokio::test]
async fn a_failing_force_to_disk_fails_like_a_failing_append() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let failing = FailingFile {
        file,
        lines_before: 1,
        failure: Failure::ForceToDisk,
    };
    let (writer, log_failed, thread) = start_writer(failing, 1024);

    assert_eq!(writer.submit(start()).await, Ok(0));
    assert_eq!(writer.submit(pass()).await, Err(LogFailed));
    assert_eq!(writer.submit(pass()).await, Err(LogFailed), "a later line");
    thread.join().expect("the Writer returns, without a panic");
    assert!(log_failed.is_cancelled());
}

#[tokio::test]
async fn a_panicking_writer_sets_log_failed() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let failing = FailingFile {
        file,
        lines_before: 0,
        failure: Failure::Panic,
    };
    let (writer, log_failed, thread) = start_writer(failing, 1024);

    assert_eq!(writer.submit(start()).await, Err(LogFailed));
    assert!(thread.join().is_err(), "the Writer panicked");
    assert!(log_failed.is_cancelled());
}

#[tokio::test]
async fn lines_queued_behind_a_panic_fail_too() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let (writer, queue) = log_queue(1024);
    let tasks: Vec<_> = (0..3)
        .map(|_| {
            let writer = writer.clone();
            tokio::spawn(async move { writer.submit(pass()).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The three lines are queued: the second panics, the third is behind it
    let file = create_log_file(folder.path(), RUN).expect("the log file is created");
    let failing = FailingFile {
        file,
        lines_before: 1,
        failure: Failure::Panic,
    };
    let log_failed = CancellationToken::new();
    let token = log_failed.clone();
    let thread = std::thread::spawn(move || run_writer(failing, queue, CLIENT.to_string(), token));
    let mut answers = Vec::new();
    for task in tasks {
        answers.push(task.await.expect("the task ends"));
    }
    let on_disk: Vec<u64> = answers.iter().filter_map(|answer| answer.ok()).collect();
    assert_eq!(on_disk, [0]);
    assert_eq!(answers.iter().filter(|answer| answer.is_err()).count(), 2);

    assert!(thread.join().is_err(), "the Writer panicked");
    assert!(log_failed.is_cancelled());
    drop(writer);
}
