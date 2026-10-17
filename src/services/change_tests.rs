use std::time::{Duration as Wait, Instant};

use chrono::Duration;

use crate::domain::change::ChangeKind;
use crate::ports::clock::Clock;
use crate::ports::node_repo::NodeRepo;
use crate::services::change_service::ChangeService;
use crate::services::error::ServiceError;
use crate::services::tests::{fixture, nv};

const NO_WAIT: Wait = Wait::ZERO;

#[tokio::test]
async fn news_that_is_already_there_comes_back_at_once() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    f.nodes
        .add_version(&s.code, None, "a.txt", nv("c1/a"), f.clock.now())
        .await
        .unwrap();
    let start = Instant::now();
    let page = f
        .changes
        .wait(&s.code, 0, Wait::from_secs(20))
        .await
        .unwrap();
    assert!(start.elapsed() < Wait::from_secs(2));
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].kind, ChangeKind::VersionAdded);
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_waiting_reader_wakes_when_a_change_commits() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let waiter = {
        let changes = f.changes.clone();
        let code = s.code.clone();
        // The service polls the log only every 60 s here, so a quick answer means the wake-up worked.
        tokio::spawn(async move { changes.wait(&code, 0, Wait::from_secs(30)).await })
    };
    tokio::time::sleep(Wait::from_millis(300)).await;
    let start = Instant::now();
    f.nodes
        .add_version(&s.code, None, "late.txt", nv("c2/late"), f.clock.now())
        .await
        .unwrap();
    let page = waiter.await.unwrap().unwrap();
    assert!(start.elapsed() < Wait::from_secs(2));
    assert_eq!(page.events[0].name.as_deref(), Some("late.txt"));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn a_quiet_wait_ends_empty_and_keeps_the_cursor() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let page = f
        .changes
        .wait(&s.code, 0, Wait::from_millis(100))
        .await
        .unwrap();
    assert_eq!((page.seq, page.reset, page.events.len()), (0, false, 0));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn only_news_counts_as_activity_not_an_idle_wait() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    let created = f.sessions.peek(&s.code).await.unwrap().last_activity_at;
    f.clock.advance(Duration::minutes(5));
    f.changes
        .wait(&s.code, 0, Wait::from_millis(50))
        .await
        .unwrap();
    assert_eq!(
        f.sessions.peek(&s.code).await.unwrap().last_activity_at,
        created
    );
    f.nodes
        .add_version(&s.code, None, "a", nv("c3/a"), f.clock.now())
        .await
        .unwrap();
    f.changes.wait(&s.code, 0, NO_WAIT).await.unwrap();
    // Postgres keeps microseconds, the test clock nanoseconds: compare with some slack.
    let touched = f.sessions.peek(&s.code).await.unwrap().last_activity_at;
    assert!(touched - created >= Duration::minutes(5) - Duration::milliseconds(1));
    f.sessions.delete(&s.code).await.unwrap();
}

#[tokio::test]
async fn unknown_and_malformed_codes_are_not_found() {
    let f = fixture().await;
    for code in ["zzzzz", "x"] {
        assert!(matches!(
            f.changes.wait(code, 0, NO_WAIT).await,
            Err(ServiceError::NotFound)
        ));
    }
}

#[tokio::test]
async fn a_session_deleted_during_a_wait_ends_it_with_not_found() {
    let f = fixture().await;
    let s = f.sessions.create("").await.unwrap();
    // Deleting a session wakes nobody, so this reader finds out on its next look at the log.
    let quick = ChangeService {
        sessions: f.sessions.clone(),
        changes: f.change_repo.clone(),
        bus: f.bus.clone(),
        poll: Wait::from_millis(100),
    };
    let waiter = tokio::spawn({
        let code = s.code.clone();
        async move { quick.wait(&code, 0, Wait::from_secs(30)).await }
    });
    tokio::time::sleep(Wait::from_millis(200)).await;
    f.sessions.delete(&s.code).await.unwrap();
    let res = tokio::time::timeout(Wait::from_secs(5), waiter)
        .await
        .expect("the wait ends")
        .unwrap();
    assert!(matches!(res, Err(ServiceError::NotFound)));
}
