//! Unit tests for the job table.

use super::*;

#[test]
fn add_job_creates_running_job() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("echo hello &".to_string());
    assert_eq!(id, 1);

    let job = mgr.get(id).unwrap();
    assert_eq!(job.id, 1);
    assert_eq!(job.command, "echo hello &");
    assert_eq!(job.state, JobState::Running);
    assert!(job.output.is_empty());
    assert!(!job.notified);
}

#[test]
fn add_multiple_jobs_increments_id() {
    let mut mgr = JobManager::new();
    let id1 = mgr.add_job("sleep 10 &".to_string());
    let id2 = mgr.add_job("find / &".to_string());
    let id3 = mgr.add_job("ls &".to_string());

    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
    assert_eq!(id3, 3);
}

#[test]
fn stop_job_transitions_running_to_stopped() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("sleep 10 &".to_string());

    assert!(mgr.stop_job(id));
    assert_eq!(mgr.get(id).unwrap().state, JobState::Stopped);
}

#[test]
fn stop_job_returns_false_for_non_running() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("sleep 10 &".to_string());
    mgr.complete_job(id, 0);

    assert!(!mgr.stop_job(id));
}

#[test]
fn stop_job_returns_false_for_invalid_id() {
    let mut mgr = JobManager::new();
    assert!(!mgr.stop_job(42));
}

#[test]
fn resume_job_transitions_stopped_to_running() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("find / &".to_string());
    mgr.stop_job(id);

    assert!(mgr.resume_job(id));
    assert_eq!(mgr.get(id).unwrap().state, JobState::Running);
}

#[test]
fn resume_job_returns_false_for_running() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("echo hi &".to_string());

    assert!(!mgr.resume_job(id));
}

#[test]
fn complete_job_transitions_to_done() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("echo hello &".to_string());

    assert!(mgr.complete_job(id, 0));
    assert_eq!(mgr.get(id).unwrap().state, JobState::Done(0));
}

#[test]
fn complete_job_with_nonzero_exit_code() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("false &".to_string());

    assert!(mgr.complete_job(id, 1));
    assert_eq!(mgr.get(id).unwrap().state, JobState::Done(1));
}

#[test]
fn complete_job_returns_false_if_already_done() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("echo x &".to_string());
    mgr.complete_job(id, 0);

    assert!(!mgr.complete_job(id, 1));
    // State unchanged.
    assert_eq!(mgr.get(id).unwrap().state, JobState::Done(0));
}

#[test]
fn complete_stopped_job() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("sleep 5 &".to_string());
    mgr.stop_job(id);

    assert!(mgr.complete_job(id, 130));
    assert_eq!(mgr.get(id).unwrap().state, JobState::Done(130));
}

#[test]
fn list_jobs_returns_all_active_jobs() {
    let mut mgr = JobManager::new();
    mgr.add_job("a".to_string());
    mgr.add_job("b".to_string());
    mgr.add_job("c".to_string());

    let jobs = mgr.list_jobs();
    assert_eq!(jobs.len(), 3);
    assert_eq!(jobs[0].command, "a");
    assert_eq!(jobs[1].command, "b");
    assert_eq!(jobs[2].command, "c");
}

#[test]
fn running_count_accuracy() {
    let mut mgr = JobManager::new();
    let id1 = mgr.add_job("a".to_string());
    let id2 = mgr.add_job("b".to_string());
    mgr.add_job("c".to_string());

    assert_eq!(mgr.running_count(), 3);

    mgr.stop_job(id1);
    assert_eq!(mgr.running_count(), 2);

    mgr.complete_job(id2, 0);
    assert_eq!(mgr.running_count(), 1);
}

#[test]
fn drain_notifications_returns_done_jobs() {
    let mut mgr = JobManager::new();
    let id1 = mgr.add_job("echo a &".to_string());
    let id2 = mgr.add_job("echo b &".to_string());
    mgr.add_job("sleep 10 &".to_string());

    mgr.complete_job(id1, 0);
    mgr.complete_job(id2, 42);

    let notifs = mgr.drain_notifications();
    assert_eq!(notifs.len(), 2);
    assert_eq!(notifs[0], (1, "echo a &".to_string(), 0));
    assert_eq!(notifs[1], (2, "echo b &".to_string(), 42));

    // Second drain returns empty -- already notified.
    let notifs2 = mgr.drain_notifications();
    assert!(notifs2.is_empty());
}

#[test]
fn has_unnotified_tracking() {
    let mut mgr = JobManager::new();
    assert!(!mgr.has_unnotified());

    let id = mgr.add_job("echo hi &".to_string());
    assert!(!mgr.has_unnotified());

    mgr.complete_job(id, 0);
    assert!(mgr.has_unnotified());

    mgr.drain_notifications();
    assert!(!mgr.has_unnotified());
}

#[test]
fn cleanup_done_removes_notified_completed_jobs() {
    let mut mgr = JobManager::new();
    let id1 = mgr.add_job("a".to_string());
    let id2 = mgr.add_job("b".to_string());
    let _id3 = mgr.add_job("c".to_string());

    mgr.complete_job(id1, 0);
    mgr.complete_job(id2, 0);
    mgr.drain_notifications();

    mgr.cleanup_done();
    // Only "c" (still running) remains.
    assert_eq!(mgr.list_jobs().len(), 1);
    assert_eq!(mgr.list_jobs()[0].command, "c");
}

#[test]
fn cleanup_done_keeps_unnotified_done_jobs() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("a".to_string());
    mgr.complete_job(id, 0);

    mgr.cleanup_done();
    // Not yet notified, so it stays.
    assert_eq!(mgr.list_jobs().len(), 1);
}

#[test]
fn append_output_accumulates() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("find / &".to_string());

    mgr.append_output(id, "line1");
    mgr.append_output(id, "line2");
    mgr.append_output(id, "line3");

    let job = mgr.get(id).unwrap();
    assert_eq!(job.output.len(), 3);
    assert_eq!(job.output[0], "line1");
    assert_eq!(job.output[1], "line2");
    assert_eq!(job.output[2], "line3");
}

#[test]
fn append_output_to_invalid_id_is_noop() {
    let mut mgr = JobManager::new();
    mgr.append_output(99, "ghost");
    assert!(mgr.is_empty());
}

#[test]
fn remove_job_returns_and_removes() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("echo done &".to_string());
    mgr.complete_job(id, 0);

    let removed = mgr.remove_job(id);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().command, "echo done &");
    assert!(mgr.is_empty());
}

#[test]
fn remove_job_returns_none_for_invalid_id() {
    let mut mgr = JobManager::new();
    assert!(mgr.remove_job(99).is_none());
}

#[test]
fn most_recent_returns_latest() {
    let mut mgr = JobManager::new();
    assert!(mgr.most_recent().is_none());

    mgr.add_job("a".to_string());
    assert_eq!(mgr.most_recent(), Some(1));

    mgr.add_job("b".to_string());
    assert_eq!(mgr.most_recent(), Some(2));
}

#[test]
fn is_empty_on_new_manager() {
    let mgr = JobManager::new();
    assert!(mgr.is_empty());
}

#[test]
fn is_empty_after_removal() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("x".to_string());
    assert!(!mgr.is_empty());

    mgr.remove_job(id);
    assert!(mgr.is_empty());
}

#[test]
fn display_formatting_running() {
    let mut mgr = JobManager::new();
    mgr.add_job("sleep 10 &".to_string());

    let job = mgr.get(1).unwrap();
    let display = format!("{job}");
    assert_eq!(display, "[1]  Running   sleep 10 &");
}

#[test]
fn display_formatting_stopped() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("find / -name \"*.rs\"".to_string());
    mgr.stop_job(id);

    let job = mgr.get(id).unwrap();
    let display = format!("{job}");
    assert_eq!(display, "[1]  Stopped   find / -name \"*.rs\"");
}

#[test]
fn display_formatting_done() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("echo hello &".to_string());
    mgr.complete_job(id, 0);

    let job = mgr.get(id).unwrap();
    let display = format!("{job}");
    assert_eq!(display, "[1]  Done(0)   echo hello &");
}

#[test]
fn parse_job_spec_percent_n() {
    let mut mgr = JobManager::new();
    mgr.add_job("a".to_string());
    mgr.add_job("b".to_string());

    assert_eq!(parse_job_spec("%1", &mgr), Some(1));
    assert_eq!(parse_job_spec("%2", &mgr), Some(2));
    assert_eq!(parse_job_spec("%3", &mgr), None);
}

#[test]
fn parse_job_spec_percent_most_recent() {
    let mut mgr = JobManager::new();
    mgr.add_job("a".to_string());
    mgr.add_job("b".to_string());

    assert_eq!(parse_job_spec("%", &mgr), Some(2));
    assert_eq!(parse_job_spec("%%", &mgr), Some(2));
    assert_eq!(parse_job_spec("%+", &mgr), Some(2));
}

#[test]
fn parse_job_spec_percent_minus() {
    let mut mgr = JobManager::new();
    mgr.add_job("a".to_string());
    mgr.add_job("b".to_string());
    mgr.add_job("c".to_string());

    assert_eq!(parse_job_spec("%-", &mgr), Some(2));
}

#[test]
fn parse_job_spec_minus_with_one_job() {
    let mut mgr = JobManager::new();
    mgr.add_job("a".to_string());

    assert_eq!(parse_job_spec("%-", &mgr), None);
}

#[test]
fn parse_job_spec_empty_manager() {
    let mgr = JobManager::new();
    assert_eq!(parse_job_spec("%", &mgr), None);
    assert_eq!(parse_job_spec("%1", &mgr), None);
    assert_eq!(parse_job_spec("%-", &mgr), None);
}

#[test]
fn parse_job_spec_invalid_format() {
    let mgr = JobManager::new();
    assert_eq!(parse_job_spec("1", &mgr), None);
    assert_eq!(parse_job_spec("", &mgr), None);
    assert_eq!(parse_job_spec("abc", &mgr), None);
    assert_eq!(parse_job_spec("%abc", &mgr), None);
}

#[test]
fn parse_job_spec_whitespace() {
    let mut mgr = JobManager::new();
    mgr.add_job("a".to_string());

    assert_eq!(parse_job_spec(" %1 ", &mgr), Some(1));
    assert_eq!(parse_job_spec("  %  ", &mgr), Some(1));
}

#[test]
fn default_trait() {
    let mgr = JobManager::default();
    assert!(mgr.is_empty());
    assert_eq!(mgr.running_count(), 0);
    assert!(!mgr.has_unnotified());
}

#[test]
fn job_state_display() {
    assert_eq!(format!("{}", JobState::Running), "Running");
    assert_eq!(format!("{}", JobState::Stopped), "Stopped");
    assert_eq!(format!("{}", JobState::Done(0)), "Done(0)");
    assert_eq!(format!("{}", JobState::Done(127)), "Done(127)");
}

#[test]
fn get_returns_none_for_missing() {
    let mgr = JobManager::new();
    assert!(mgr.get(1).is_none());
    assert!(mgr.get(0).is_none());
}

#[test]
fn get_mut_allows_modification() {
    let mut mgr = JobManager::new();
    let id = mgr.add_job("test".to_string());

    if let Some(job) = mgr.get_mut(id) {
        job.command = "modified".to_string();
    }

    assert_eq!(mgr.get(id).unwrap().command, "modified");
}

#[test]
fn ids_do_not_recycle_after_removal() {
    let mut mgr = JobManager::new();
    let id1 = mgr.add_job("a".to_string());
    mgr.remove_job(id1);

    let id2 = mgr.add_job("b".to_string());
    assert_eq!(id2, 2); // Not 1 again.
}

#[test]
fn full_lifecycle() {
    let mut mgr = JobManager::new();

    // Create and run.
    let id = mgr.add_job("sleep 5 &".to_string());
    assert_eq!(mgr.running_count(), 1);
    assert!(!mgr.has_unnotified());

    // Accumulate output.
    mgr.append_output(id, "tick 1");
    mgr.append_output(id, "tick 2");

    // Stop.
    assert!(mgr.stop_job(id));
    assert_eq!(mgr.running_count(), 0);
    assert_eq!(mgr.get(id).unwrap().state, JobState::Stopped);

    // Resume.
    assert!(mgr.resume_job(id));
    assert_eq!(mgr.running_count(), 1);

    // Complete.
    assert!(mgr.complete_job(id, 0));
    assert_eq!(mgr.running_count(), 0);
    assert!(mgr.has_unnotified());

    // Drain notifications.
    let notifs = mgr.drain_notifications();
    assert_eq!(notifs.len(), 1);
    assert_eq!(notifs[0].0, id);
    assert!(!mgr.has_unnotified());

    // Cleanup.
    mgr.cleanup_done();
    assert!(mgr.is_empty());
}
