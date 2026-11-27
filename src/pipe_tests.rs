#[cfg(test)]
mod t {
    use super::*;

    #[tokio::test]
    async fn scheduler_ci_hold_rechecks_marker_after_remote_observation() {
        for mutation in ["none", "replace", "remove", "malformed", "db_error"] {
            let a = std::sync::Arc::new(crate::app::App::new(
                crate::db::Db::mem(),
                std::env::temp_dir(),
                1,
                "x".into(),
            ));
            a.db.ks("cired:o/r", &json!({"sha":"a".repeat(40)}));
            let held = red_with(&a, "o/r", "local", || {
                match mutation {
                    "replace" => a.db.ks("cired:o/r", &json!({"sha":"b".repeat(40)})),
                    "remove" => a.db.kd("cired:o/r"),
                    "malformed" => {
                        a.db.run("update kv set v='{' where k='cired:o/r'", pa![]);
                    }
                    "db_error" => {
                        a.db.run("drop table kv", pa![]);
                    }
                    _ => {}
                }
                std::future::ready(json!({"s":"b".repeat(40)}))
            })
            .await;
            assert_eq!(held, mutation != "none", "{mutation}");
        }
    }

    #[tokio::test]
    async fn scheduler_cache_is_bound_to_exact_holds_and_latest_generation() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let first = json!({"sha":"a".repeat(40),"t":1});
        let head = json!({"s":"b".repeat(40)});
        a.db.ks("cired:o/r", &first);
        assert!(!red_with(&a, "o/r", "local", || std::future::ready(head.clone())).await);
        assert!(!red_with(&a, "o/r", "local", || {
            panic!("unchanged snapshot should use cached proof");
            #[allow(unreachable_code)]
            std::future::ready(Value::Null)
        }).await);
        // Same failing SHA with a new failure receipt must be observed again.
        a.db.ks("cired:o/r", &json!({"sha":"a".repeat(40),"t":2}));
        assert!(red_with(&a, "o/r", "local", || std::future::ready(json!({"s":"a".repeat(40)}))).await);
        // A generic manual observation cannot authorize a scheduler hold.
        let manual = crate::gh::begin_head_observation(&a, "o/r");
        assert!(crate::gh::publish_head_observation(&a, "o/r", manual, &head));
        assert!(red_with(&a, "o/r", "local", || std::future::ready(Value::Null)).await);
        // A later in-flight observation supersedes the remote result.
        assert!(red_with(&a, "o/r", "local", || {
            crate::gh::begin_head_observation(&a, "o/r");
            std::future::ready(head.clone())
        }).await);
        assert!(!red_with(&a, "o/r", "local", || std::future::ready(head.clone())).await);
    }

    #[tokio::test]
    async fn failed_ci_receipt_write_blocks_admission_until_persisted() {
        let path = std::env::temp_dir().join(format!("apollo-ci-receipt-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(), path.clone(), 1, "x".into(),
        ));
        a.db.run("insert into rp(full,ci) values('o/r','true'),('o/other','true')", pa![]);
        a.db.run("insert into jobs(id,repo,kind,st) values(1,'o/r','fix','ready'),(2,'o/other','fix','ready')", pa![]);
        let marker = json!({"sha":"a".repeat(40),"t":1});
        a.db.0.lock().unwrap().execute_batch(
            "create trigger fail_ci_receipt before insert on kv
             when new.k='cired:o/r' begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert!(record_ci_failure(&a, "o/r", "", &marker).is_err());
        assert!(a.db.kg("cired:o/r").is_null());
        assert!(red_with(&a, "o/r", "local", || std::future::ready(Value::Null)).await);
        assert!(!admit(&a, "o/r", "local", 1));
        assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 0);
        assert!(a.lk.lock().unwrap().is_empty());
        assert!(clear_prepared_ci_markers_with(&a, "o/r", "local", &"b".repeat(40), || {
            std::future::ready(json!({"s":"b".repeat(40)}))
        }).await.is_err());
        assert!(admit(&a, "o/other", "local", 2));
        a.db.run("drop trigger fail_ci_receipt", pa![]);
        assert!(red_with(&a, "o/r", "local", || std::future::ready(json!({"s":"a".repeat(40)}))).await);
        assert_eq!(a.db.kg("cired:o/r"), marker);
        assert!(a.pending_ci_holds.lock().unwrap().is_empty());
        assert!(!admit(&a, "o/r", "local", 1));
        // A fresh observation is required after the cache's current HEAD
        // matches the failed SHA; invalidate it explicitly for this control.
        crate::gh::begin_head_observation(&a, "o/r");
        assert!(!red_with(&a, "o/r", "local", || std::future::ready(json!({"s":"b".repeat(40)}))).await);
        assert!(record_ci_admission(&a, "o/r", "", 1, || {}).unwrap());
        assert!(!ci_storage_pending(&a, "o/r"));
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn ci_hold_journal_survives_app_restart_and_rejects_corruption() {
        let path = std::env::temp_dir().join(format!("apollo-ci-journal-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("state.db");
        let marker = json!({"sha":"a".repeat(40),"t":1});
        let scoped = json!({"sha":"b".repeat(40),"t":2});
        {
            let db = crate::db::Db::open(&file).unwrap();
            db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
            db.run("insert into jobs(id,repo,kind,st) values(1,'o/r','fix','ready')", pa![]);
            let a = std::sync::Arc::new(crate::app::App::new(db, path.clone(), 1, "x".into()));
            a.db.run("create trigger fail_ci_journal before insert on kv when new.k glob 'cired:*' begin select raise(abort,'injected'); end", pa![]);
            assert!(record_ci_failure(&a, "o/r", "", &marker).is_err());
            assert!(record_ci_failure(&a, "o/r", "@host", &scoped).is_err());
            assert!(ci_hold_journal(&a, "o/r").is_file());
            assert!(a.db.kg("cired:o/r").is_null());
        }
        {
            let db = crate::db::Db::open(&file).unwrap();
            let a = std::sync::Arc::new(crate::app::App::new(db, path.clone(), 1, "x".into()));
            assert!(a.pending_ci_holds.lock().unwrap().is_empty());
            assert!(ci_storage_pending(&a, "o/r"));
            assert!(!admit(&a, "o/r", "local", 1));
            assert!(a.lk.lock().unwrap().is_empty());
            a.db.run("drop trigger fail_ci_journal", pa![]);
            assert!(red_with(&a, "o/r", "local", || std::future::ready(json!({"s":"a".repeat(40)}))).await);
            assert_eq!(a.db.kg("cired:o/r"), marker);
            assert_eq!(a.db.kg("cired:o/r@host"), scoped);
            assert!(!ci_storage_pending(&a, "o/r"));
            assert!(!admit(&a, "o/r", "local", 1));
            // A damaged journal is not evidence that the repo is safe.
            let journal = ci_hold_journal(&a, "o/r");
            std::fs::write(&journal, b"{broken").unwrap();
            assert!(ci_storage_pending(&a, "o/r"));
            assert!(red_with(&a, "o/r", "local", || std::future::ready(json!({"s":"c".repeat(40)}))).await);
            assert!(!admit(&a, "o/r", "local", 1));
            assert_eq!(std::fs::read(&journal).unwrap(), b"{broken");
            assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 0);
            std::fs::remove_file(journal).unwrap();
            crate::gh::begin_head_observation(&a, "o/r");
            assert!(!red_with(&a, "o/r", "local", || std::future::ready(json!({"s":"c".repeat(40)}))).await);
            assert!(record_ci_admission(&a, "o/r", "", 1, || {}).unwrap());
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn identical_ci_hold_recreation_invalidates_cached_and_inflight_evidence() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let marker = json!({"sha":"a".repeat(40),"t":1});
        let head = json!({"s":"b".repeat(40)});
        a.db.ks("cired:o/r", &marker);
        assert!(!red_with(&a, "o/r", "local", || std::future::ready(head.clone())).await);
        a.db.ks("cired:o/r", &marker);
        assert!(red_with(&a, "o/r", "local", || std::future::ready(Value::Null)).await);
        assert!(red_with(&a, "o/r", "local", || {
            a.db.kd("cired:o/r");
            a.db.ks("cired:o/r", &marker);
            std::future::ready(head.clone())
        }).await);
        assert!(!clear_prepared_ci_markers_with(&a, "o/r", "local", &"b".repeat(40), || {
            a.db.kd("cired:o/r");
            a.db.ks("cired:o/r", &marker);
            std::future::ready(head.clone())
        }).await.unwrap());
        assert_eq!(a.db.kg("cired:o/r"), marker);
        a.db.run("delete from ci_hold_epochs where k='cired:o/r'", pa![]);
        assert!(red_with(&a, "o/r", "local", || std::future::ready(head.clone())).await);
        assert!(clear_prepared_ci_markers_with(&a, "o/r", "local", &"b".repeat(40), || {
            std::future::ready(head.clone())
        }).await.is_err());
        assert_eq!(a.db.kg("cired:o/r"), marker);
        a.db.ks("cired:o/r", &marker);
        assert!(!red_with(&a, "o/r", "local", || std::future::ready(head.clone())).await);
        assert!(clear_prepared_ci_markers_with(&a, "o/r", "local", &"b".repeat(40), || {
            std::future::ready(head.clone())
        }).await.unwrap());
        assert!(a.db.kg("cired:o/r").is_null());
    }

    #[test]
    fn ci_hold_snapshot_includes_global_and_host_markers_and_rejects_invalid_json() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        assert!(ci_markers(&a, "o/r", "@host").unwrap().is_empty());
        a.db.ks("cired:o/r", &json!({"sha":"a".repeat(40)}));
        a.db.ks("cired:o/r@host", &json!({"sha":"b".repeat(40)}));
        a.db.ks("cired:o/r@other", &json!({"sha":"c".repeat(40)}));
        let markers = ci_markers(&a, "o/r", "@host").unwrap();
        assert_eq!(markers.len(), 2);
        assert!(!markers.iter().all(|marker| ci_marker_superseded(
            &json!({"s":"b".repeat(40)}),
            marker
        )));
        assert_eq!(ci_markers(&a, "o/r", "").unwrap().len(), 1);
        a.db.run("update kv set v='{' where k='cired:o/r@host'", pa![]);
        assert!(ci_markers(&a, "o/r", "@host").is_err());
        a.db.run("drop table kv", pa![]);
        assert!(ci_markers(&a, "o/r", "").is_err());
    }

    #[test]
    fn ci_hold_cleanup_is_atomic_and_fail_closed() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let clear = |base: &str| {
            let expected = ci_marker_rows(&a.db.0.lock().unwrap(), "o/r", "@host")?;
            let generation = crate::gh::begin_head_observation(&a, "o/r");
            clear_superseded_ci_markers(&a, "o/r", "@host", base, &expected, Some(generation))
        };
        let base = "b".repeat(40);
        let old = json!({"sha":"a".repeat(40)});
        let current = json!({"sha":base});
        a.db.ks("cired:o/r", &old);
        a.db.ks("cired:o/r@host", &current);
        a.db.ks("cired:o/r@other", &old);
        assert!(!clear(&base).unwrap());
        assert_eq!(a.db.kg("cired:o/r"), old);
        assert_eq!(a.db.kg("cired:o/r@host"), current);
        a.db.ks("cired:o/r@host", &json!({"sha":"invalid"}));
        assert!(!clear(&base).unwrap());
        assert_eq!(a.db.kg("cired:o/r"), old);
        a.db.run("update kv set v='{' where k='cired:o/r@host'", pa![]);
        assert!(clear(&base).is_err());
        assert_eq!(a.db.kg("cired:o/r"), old);
        a.db.ks("cired:o/r@host", &old);
        assert!(!clear("invalid").unwrap());
        a.db.0.lock().unwrap().execute_batch(
            "create trigger fail_hold_delete before delete on kv
             when old.k='cired:o/r@host' begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert!(clear(&base).is_err());
        assert_eq!(a.db.kg("cired:o/r"), old);
        assert_eq!(a.db.kg("cired:o/r@host"), old);
        a.db.run("drop trigger fail_hold_delete", pa![]);
        assert!(clear(&base).unwrap());
        assert!(a.db.kg("cired:o/r").is_null());
        assert!(a.db.kg("cired:o/r@host").is_null());
        assert_eq!(a.db.kg("cired:o/r@other"), old);
        assert!(clear(&base).unwrap());
    }

    #[tokio::test]
    async fn prepared_ci_cleanup_requires_fresh_default_head_and_unchanged_holds() {
        for mutation in ["none", "stale_base", "invalid_head", "new_hold", "removed_hold", "newer_head", "pending_head"] {
            let a = std::sync::Arc::new(crate::app::App::new(
                crate::db::Db::mem(),
                std::env::temp_dir(),
                1,
                "x".into(),
            ));
            let base = "b".repeat(40);
            let old = json!({"sha":"a".repeat(40)});
            let newer = json!({"sha":"c".repeat(40)});
            a.db.ks("cired:o/r", &old);
            let cleared = clear_prepared_ci_markers_with(&a, "o/r", "local", &base, || {
                let head = match mutation {
                    "stale_base" => json!({"s":"c".repeat(40)}),
                    "invalid_head" => json!({"s":"invalid"}),
                    "new_hold" => {
                        a.db.ks("cired:o/r", &newer);
                        json!({"s":base})
                    }
                    "removed_hold" => {
                        a.db.kd("cired:o/r");
                        json!({"s":base})
                    }
                    "newer_head" => {
                        let newer = crate::gh::begin_head_observation(&a, "o/r");
                        assert!(crate::gh::publish_head_observation(
                            &a, "o/r", newer, &json!({"s":"c".repeat(40)})
                        ));
                        json!({"s":base})
                    }
                    "pending_head" => {
                        crate::gh::begin_head_observation(&a, "o/r");
                        json!({"s":base})
                    }
                    _ => json!({"s":base}),
                };
                std::future::ready(head)
            })
            .await
            .unwrap();
            assert_eq!(cleared, mutation == "none", "{mutation}");
            let expected = match mutation {
                "none" | "removed_hold" => Value::Null,
                "new_hold" => newer,
                _ => old,
            };
            assert_eq!(a.db.kg("cired:o/r"), expected);
        }
    }

    #[test]
    fn final_admission_requires_current_ci_proof_and_committed_history() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        a.db.run("insert into rp(full,on_,ci) values('o/r',1,'true')", pa![]);
        a.db.run(
            "insert into jobs(id,repo,kind,st) values(1,'o/r','fix','ready')",
            pa![],
        );
        let reserved = std::cell::Cell::new(0);
        let admit = || {
            record_ci_admission(&a, "o/r", "@host", 1, || {
                reserved.set(reserved.get() + 1);
            })
        };
        let head = json!({"s":"b".repeat(40)});
        a.db.ks("cired:o/r", &json!({"sha":"a".repeat(40),"t":1}));
        let rows = ci_marker_rows(&a.db.0.lock().unwrap(), "o/r", "@host").unwrap();
        let generation = crate::gh::begin_head_observation(&a, "o/r");
        assert!(crate::gh::publish_ci_head_observation(&a, "o/r", generation, &head, &rows));
        // A hold written after pick must block final admission.
        a.db.ks("cired:o/r@host", &json!({"sha":"b".repeat(40),"t":2}));
        assert!(!admit().unwrap());
        assert_eq!(reserved.get(), 0);
        assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 0);
        a.db.kd("cired:o/r@host");
        crate::gh::begin_head_observation(&a, "o/r");
        assert!(!admit().unwrap());
        let generation = crate::gh::begin_head_observation(&a, "o/r");
        assert!(crate::gh::publish_ci_head_observation(&a, "o/r", generation, &head, &rows));
        a.db.0.lock().unwrap().execute_batch(
            "create trigger fail_admit before insert on job_schedule
             begin select raise(abort,'injected'); end;",
        ).unwrap();
        assert!(admit().is_err());
        assert_eq!(reserved.get(), 0);
        a.db.run("drop trigger fail_admit", pa![]);
        assert!(admit().unwrap());
        assert_eq!(reserved.get(), 1);
        assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 1);
        // Verified/pinned work preserves its existing exemption.
        a.db.ks("cired:o/r", &json!({"sha":"b".repeat(40)}));
        a.db.run("update jobs set st='submit' where id=1", pa![]);
        assert!(admit().unwrap());
        assert_eq!(reserved.get(), 2);
        a.db.run("update jobs set st='drop' where id=1", pa![]);
        assert!(admit().is_err());
        assert_eq!(reserved.get(), 2);
    }

    #[test]
    fn final_admission_rechecks_repository_policy_inside_transaction() {
        let a = std::sync::Arc::new(crate::app::App::new(
            crate::db::Db::mem(),
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        a.db.run(
            "insert into rp(full,on_,ci,hunt) values('o/r',1,'true',1)",
            pa![],
        );
        a.db.run(
            "insert into jobs(id,repo,kind,st) values(1,'o/r','fix','ready'),(2,'o/r','hunt','ready')",
            pa![],
        );
        let reserved = std::cell::Cell::new(0);
        let admit = |id| {
            record_ci_admission(&a, "o/r", "", id, || {
                reserved.set(reserved.get() + 1);
            })
        };
        for revoke in [
            "update rp set on_=0",
            "update rp set on_=null",
            "update rp set ci=null",
            "update rp set ci=' '",
            "update rp set ci=char(9)||char(10)",
        ] {
            a.db.run(revoke, pa![]);
            assert!(!admit(1).unwrap(), "{revoke}");
            assert!(!admit(2).unwrap(), "{revoke}");
            assert_eq!(reserved.get(), 0);
            assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 0);
            a.db.run("update rp set on_=1,ci='true'", pa![]);
        }
        a.db.run("update rp set hunt=0", pa![]);
        assert!(!admit(2).unwrap());
        assert!(admit(1).unwrap());
        a.db.run("update rp set hunt=1", pa![]);
        assert!(admit(2).unwrap());
        a.db.run("update rp set on_=0,ci=''", pa![]);
        a.db.run("update jobs set st='submit' where id=1", pa![]);
        assert!(admit(1).unwrap());
        a.db.run("update jobs set d=json_object('rp',json('true')) where id=2", pa![]);
        assert!(!admit(2).unwrap());
        assert_eq!(reserved.get(), 3);
    }

    #[test]
    fn disk_full_admission_does_not_reserve_lane_or_persist_history() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
        db.run("insert into hs(id,ln,on_) values('local',2,1)", pa![]);
        let id = db.jnew("o/r", 1, "fix", "fixture", "", "ready", "", json!({}), None, 0.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db, std::env::temp_dir(), 1, "x".into(),
        ));
        {
            let c = a.db.0.lock().unwrap();
            c.execute_batch(
                "create table admission_pressure(payload blob);
                 create trigger admission_disk_full after insert on job_schedule
                 begin insert into admission_pressure values(zeroblob(1048576)); end;",
            ).unwrap();
            let pages: i64 = c.pragma_query_value(None, "page_count", |row| row.get(0)).unwrap();
            c.pragma_update(None, "max_page_count", pages).unwrap();
        }
        let error = record_ci_admission(&a, "o/r", "", id, || panic!("reserved on failed write"))
            .unwrap_err();
        assert_eq!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DiskFull));
        assert!(!admit(&a, "o/r", "local", id));
        assert!(a.act.lock().unwrap().is_empty());
        assert!(a.lk.lock().unwrap().is_empty());
        assert_eq!(a.db.jg(id).unwrap().st, "ready");
        assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 0);
        assert_eq!(a.db.n("select count(*) from admission_pressure", pa![]), 0);
        a.db.run("drop trigger admission_disk_full", pa![]);
        a.db.0.lock().unwrap().pragma_update(None, "max_page_count", 1_000_000).unwrap();
        assert!(admit(&a, "o/r", "local", id));
        assert!(a.act.lock().unwrap().contains("o/r local"));
        assert!(a.lk.lock().unwrap().contains(&id));
        assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 1);
    }

    #[test]
    fn busy_admission_rechecks_hold_committed_by_independent_writer() {
        let path = std::env::temp_dir().join(format!("apollo-admit-busy-{}", rand::random::<u64>()));
        std::fs::create_dir(&path).unwrap();
        let file = path.join("state.db");
        {
            let db = crate::db::Db::open(&file).unwrap();
            db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
            db.run("insert into hs(id,ln,on_) values('local',2,1)", pa![]);
            let id = db.jnew("o/r", 1, "fix", "fixture", "", "ready", "", json!({}), None, 0.0);
            db.0.lock().unwrap().busy_timeout(std::time::Duration::ZERO).unwrap();
            let a = std::sync::Arc::new(crate::app::App::new(
                db, path.clone(), 1, "x".into(),
            ));
            let mut other = rusqlite::Connection::open(&file).unwrap();
            let tx = other.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            tx.execute("insert into kv(k,v) values(?1,?2)", rusqlite::params![
                "cired:o/r", json!({"sha":"a".repeat(40),"t":1}).to_string()
            ]).unwrap();
            let error = record_ci_admission(&a, "o/r", "", id, || panic!("reserved while busy"))
                .unwrap_err();
            assert_eq!(error.sqlite_error_code(), Some(rusqlite::ErrorCode::DatabaseBusy));
            assert!(!admit(&a, "o/r", "local", id));
            assert!(a.act.lock().unwrap().is_empty());
            assert!(a.lk.lock().unwrap().is_empty());
            tx.commit().unwrap();
            assert!(!admit(&a, "o/r", "local", id));
            assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 0);
            assert!(a.act.lock().unwrap().is_empty());
            assert!(a.lk.lock().unwrap().is_empty());
            let suffix = hk(&a, "local");
            let rows = ci_marker_rows(&a.db.0.lock().unwrap(), "o/r", &suffix).unwrap();
            let generation = crate::gh::begin_head_observation(&a, "o/r");
            assert!(crate::gh::publish_ci_head_observation(
                &a, "o/r", generation, &json!({"s":"b".repeat(40)}), &rows
            ));
            assert!(admit(&a, "o/r", "local", id));
            assert_eq!(a.db.n("select count(*) from job_schedule", pa![]), 1);
            assert!(a.lk.lock().unwrap().contains(&id));
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn removed_repository_cannot_admit_stale_submission() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into jobs(id,repo,kind,st) values(1,'o/r','fix','submit')",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        assert!(!admit(&a, "o/r", "local", 1));
        assert!(a.lk.lock().unwrap().is_empty());
    }

    #[test]
    fn cancellation_state_is_strict_and_database_errors_defer_work() {
        let db = crate::db::Db::mem();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let j = Job::of(json!({"id":42,"kind":"fix","d":{}}));
        assert!(kill(&a, &j).is_ok());
        a.db.ks("kill:42", &json!(true));
        assert!(matches!(kill(&a, &j), Err(X::Drop(_))));
        a.db.ks("kill:42", &json!("invalid"));
        assert!(matches!(kill(&a, &j), Err(X::Defer(_))));
        a.db.run("drop table kv", pa![]);
        assert!(matches!(kill(&a, &j), Err(X::Defer(_))));
    }

    #[test]
    fn approval_publication_preserves_cancellation_and_complete_preview() {
        let db = crate::db::Db::mem();
        db.run("insert into jobs(id,st,kind) values(1,'ready','fix')", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        let mut j = a.db.jg(1).unwrap();
        j.d["pt"] = json!("complete preview");
        a.db.ks("kill:1", &json!(true));
        assert!(matches!(publish_approval(&a, &j, "waiting"), Err(X::Drop(_))));
        assert_eq!(a.db.jg(1).unwrap().st, "ready");
        a.db.kd("kill:1");
        assert!(publish_approval(&a, &j, "waiting").is_ok());
        assert_eq!(a.db.jg(1).unwrap().st, "approve");
        assert_eq!(a.db.jg(1).unwrap().d["pt"], "complete preview");
    }

    #[test]
    fn startup_recovery_is_atomic_and_preserves_approved_submission() {
        let db = crate::db::Db::mem();
        db.0.lock().unwrap().execute_batch(
            "insert into jobs(id,st,kind,d) values
             (1,'fix','fix','{\"h\":\"local\"}'),
             (2,'ci','fix','{\"h\":\"local\",\"br\":\"fix/2\"}'),
             (3,'submit','fix','{\"ap\":true,\"h\":\"local\",\"br\":\"fix/3\"}'),
             (4,'done','fix','{}');
             create trigger fail_recovery before update on jobs when old.id=2 begin select raise(abort,'injected'); end;",
        ).unwrap();
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        assert!(recover(&a).is_err());
        assert_eq!(a.db.jg(1).unwrap().st, "fix");
        assert_eq!(a.db.jg(2).unwrap().st, "ci");
        a.db.0
            .lock()
            .unwrap()
            .execute_batch("drop trigger fail_recovery")
            .unwrap();
        a.db.run("update jobs set d='invalid' where id=2", pa![]);
        assert!(recover(&a).is_err());
        assert_eq!(a.db.jg(1).unwrap().st, "fix");
        a.db.run(
            "update jobs set d=? where id=2",
            pa![json!({"h":"local","br":"fix/2"}).to_string()],
        );
        recover(&a).unwrap();
        let prep = a.db.jg(1).unwrap();
        let verified = a.db.jg(2).unwrap();
        assert_eq!(prep.st, "ready");
        assert_eq!(prep.d["rp"], true);
        assert_eq!(verified.st, "ready");
        assert_eq!(verified.d["rs"], true);
        assert_eq!(a.db.jg(3).unwrap().st, "submit");
        assert_eq!(a.db.jg(3).unwrap().d["ap"], true);
        assert_eq!(a.db.jg(4).unwrap().st, "done");
        recover(&a).unwrap();
        assert_eq!(a.db.jg(1).unwrap().d, prep.d);
        assert_eq!(a.db.jg(2).unwrap().d, verified.d);
    }

    #[test]
    fn coderabbit_review_prompt_never_claims_maintainer_authority() {
        let mut job = Job::of(json!({"kind":"review","d":{"coderabbit_source":"coderabbit"}}));
        let (authority, action) = review_prompt_policy(&job);
        assert!(authority.contains("untrusted advisory findings"));
        assert!(action.contains("Independently verify"));
        assert!(!authority.contains("A maintainer requested"));
        job.d = json!({"steward_review_id":"coderabbit"});
        assert_eq!(review_prompt_policy(&job), (authority, action));
        job.d = json!({"steward_review_id":42});
        assert_eq!(review_prompt_policy(&job).0, "A maintainer requested a change:");
    }

    #[test]
    fn automated_review_authorization_is_bound_to_current_policy_and_finding_epoch() {
        let db = crate::db::Db::mem();
        let head = "a".repeat(40);
        db.run(
            "insert into rp(full,on_,coderabbit_repair,coderabbit_epoch,steward_repair) values('o/r',1,1,4,1)",
            vec![],
        );
        db.run(
            "insert into prs(repo,num,st) values('o/r',1,'open')",
            vec![],
        );
        let mut job = Job::of(json!({"id":7,"repo":"o/r","pr":1,"kind":"review","d":{
            "steward_head":head,"coderabbit_source":"coderabbit","coderabbit_comment_ids":[91]
        }}));
        assert!(automated_review_authorized(&db, &job).is_err());
        db.run(
            "insert into coderabbit_findings(repo,number,comment_id,review_id,head,state,severity,body_hash,activation,job,created_at,updated_at) values('o/r',1,91,5,?,'queued','high','hash',4,7,0,0)",
            pa![head.clone()],
        );
        assert!(automated_review_authorized(&db, &job).is_ok());
        job.d["coderabbit_comment_ids"] = json!([91,91]);
        assert!(automated_review_authorized(&db, &job).is_err());
        job.d["coderabbit_comment_ids"] = json!([91,92]);
        assert!(automated_review_authorized(&db, &job).is_err());
        job.d["coderabbit_comment_ids"] = json!([91]);
        db.run("update rp set coderabbit_repair=0,coderabbit_epoch=5", vec![]);
        assert!(automated_review_authorized(&db, &job).is_err());
        db.run("update rp set coderabbit_repair=1,coderabbit_epoch=6", vec![]);
        assert!(automated_review_authorized(&db, &job).is_err());
        job.d = json!({"steward_head":head,"steward_review_id":91});
        assert!(automated_review_authorized(&db, &job).is_err());
        let policy = db.one("select steward_activation from rp", vec![]).unwrap();
        job.d["steward_activation"] = policy["steward_activation"].clone();
        assert!(automated_review_authorized(&db, &job).is_ok());
        db.run("update rp set steward_repair=0", vec![]);
        assert!(automated_review_authorized(&db, &job).is_err());
        db.run("update rp set steward_repair=1", vec![]);
        assert!(automated_review_authorized(&db, &job).is_err());
        job.d["steward_activation"] =
            db.one("select steward_activation from rp", vec![]).unwrap()["steward_activation"].clone();
        assert!(automated_review_authorized(&db, &job).is_ok());
        db.run("delete from rp", vec![]);
        db.run("insert into rp(full,on_,steward_repair) values('o/r',1,1)", vec![]);
        assert!(automated_review_authorized(&db, &job).is_err());
        job.d = json!({});
        assert!(automated_review_authorized(&db, &job).is_ok());
    }

    #[test]
    fn remote_branch_inventory_requires_exact_single_valid_ref(){
        let sha="a".repeat(40);
        let good=format!("{sha}\trefs/heads/fix/1");
        assert_eq!(remote_branch_sha(&good,"fix/1").unwrap(),Some(sha.clone()));
        assert_eq!(remote_branch_sha("","fix/1").unwrap(),None);
        for bad in [format!("{good}\n{good}"),format!("{sha}\trefs/heads/fix/2"),"bad\trefs/heads/fix/1".to_string(),format!("{sha} refs/heads/fix/1")] {
            assert!(remote_branch_sha(&bad,"fix/1").is_err());
        }
    }

    #[test]
    fn publication_branch_lease_never_overwrites_unrelated_fix(){
        let old="a".repeat(40);
        let candidate="b".repeat(40);
        let other="c".repeat(40);
        assert_eq!(submit_branch_lease("fix","fix/1",None,&candidate,"").unwrap().as_deref(),Some("--force-with-lease=refs/heads/fix/1:"));
        assert_eq!(submit_branch_lease("fix","fix/1",Some(&candidate),&candidate,"").unwrap(),None);
        assert!(submit_branch_lease("fix","fix/1",Some(&old),&candidate,&old).is_err());
        for kind in ["review","rebase"] {
            assert_eq!(submit_branch_lease(kind,"fix/1",Some(&old),&candidate,&old).unwrap(),Some(format!("--force-with-lease=refs/heads/fix/1:{old}")));
            assert_eq!(submit_branch_lease(kind,"fix/1",Some(&candidate),&candidate,&old).unwrap(),None);
            assert!(submit_branch_lease(kind,"fix/1",None,&candidate,&old).is_err());
            assert!(submit_branch_lease(kind,"fix/1",Some(&other),&candidate,&old).is_err());
            assert!(submit_branch_lease(kind,"fix/1",Some(&old),&candidate,"").is_err());
        }
        assert!(submit_branch_lease("fix","fix/1",None,"invalid","").is_err());
    }

    #[test]
    fn actual_git_lease_rejects_branch_created_or_changed_after_observation(){
        let root=std::env::temp_dir().join(format!("apollo-lease-{}",hex::encode(rand::random::<[u8;8]>())));
        std::fs::create_dir_all(&root).unwrap();
        let git=|args:&[&str]| std::process::Command::new("git").args(args).current_dir(&root).output().unwrap();
        let ok=|args:&[&str]| {
            let out=git(args);
            assert!(out.status.success(),"{args:?}: {}",String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        ok(&["init","-q","-b","main","work"]);
        ok(&["init","-q","--bare","remote.git"]);
        ok(&["-C","work","-c","user.name=fixture","-c","user.email=fixture@example.test","commit","-q","--allow-empty","-m","first"]);
        let first=ok(&["-C","work","rev-parse","HEAD"]);
        ok(&["-C","work","-c","user.name=fixture","-c","user.email=fixture@example.test","commit","-q","--allow-empty","-m","second"]);
        let second=ok(&["-C","work","rev-parse","HEAD"]);
        let branch="fix/1";
        let target="refs/heads/fix/1";
        let absent=remote_branch_sha(&ok(&["-C","work","ls-remote","--refs","../remote.git",target]),branch).unwrap();
        assert!(absent.is_none());
        let lease=submit_branch_lease("fix",branch,absent.as_deref(),&second,"").unwrap().unwrap();
        ok(&["-C","work","push","-q","../remote.git",&format!("{first}:{target}")]);
        assert!(!git(&["-C","work","push","-q",&lease,"../remote.git",&format!("{second}:{target}")]).status.success());
        assert_eq!(ok(&["--git-dir=remote.git","rev-parse",target]),first);
        let lease=submit_branch_lease("review",branch,Some(&first),&second,&first).unwrap().unwrap();
        ok(&["-C","work","push","-q",&lease,"../remote.git",&format!("{second}:{target}")]);
        assert!(!git(&["-C","work","push","-q",&lease,"../remote.git",&format!("{first}:{target}")]).status.success());
        assert_eq!(ok(&["--git-dir=remote.git","rev-parse",target]),second);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn steward_rebase_requires_exact_base_at_execution_and_publication() {
        let sha = "a".repeat(40);
        let mut job = Job::of(json!({"kind":"rebase","d":{
            "steward_rebase_base_ref":"main","steward_rebase_base_sha":sha
        }}));
        assert!(rebase_base_matches(&job, "main", &sha).is_ok());
        assert!(rebase_base_matches(&job, "release", &sha).is_err());
        assert!(rebase_base_matches(&job, "main", &"b".repeat(40)).is_err());
        assert!(rebase_base_matches(&job, "main", "invalid").is_err());
        job.d["steward_rebase_base_sha"] = Value::Null;
        assert!(rebase_base_matches(&job, "main", &sha).is_err());
        job.d.as_object_mut().unwrap().remove("steward_rebase_base_sha");
        assert!(rebase_base_matches(&job, "main", &sha).is_err());
        job.d = json!({});
        assert!(rebase_base_matches(&job, "main", &sha).is_ok());
    }

    #[test]
    fn review_publication_requires_original_open_pr_and_exact_head(){
        let source="a".repeat(40);
        let candidate="b".repeat(40);
        let mut pr=json!({"state":"open","head":{"repo":{"full_name":"o/r"},"ref":"fix","sha":source}});
        assert!(review_publish_target_matches(&pr,"o/r","fix",&source,&candidate));
        pr["head"]["sha"]=json!(candidate);
        assert!(review_publish_target_matches(&pr,"o/r","fix",&source,&candidate));
        pr["head"]["sha"]=json!("c".repeat(40));
        assert!(!review_publish_target_matches(&pr,"o/r","fix",&source,&candidate));
        pr["head"]["sha"]=json!(source);
        assert!(!review_publish_target_matches(&pr,"other/r","fix",&source,&candidate));
        assert!(!review_publish_target_matches(&pr,"o/r","other",&source,&candidate));
        pr["state"]=json!("closed");
        assert!(!review_publish_target_matches(&pr,"o/r","fix",&source,&candidate));
    }

    #[test]
    fn review_checkout_matches_bound_head_including_steward_and_legacy(){
        let head="a".repeat(40);
        let other="b".repeat(40);
        let mut j=Job::of(json!({"kind":"review","d":{"source_head":head}}));
        assert!(check_review_checkout(&j,&head).is_ok());
        assert!(check_review_checkout(&j,&other).is_err());
        j.d["steward_head"]=json!(other);
        assert!(check_review_checkout(&j,&head).is_err());
        j.d=json!({"steward_head":head});
        assert!(check_review_checkout(&j,&head).is_ok());
        j.d=json!({});
        assert!(check_review_checkout(&j,&other).is_ok());
        j.d=json!({"source_head":"invalid"});
        assert!(check_review_checkout(&j,&other).is_err());
    }

    #[test]
    fn watcher_rebase_requires_its_observed_head(){
        let head="a".repeat(40);
        let other="b".repeat(40);
        let mut j=Job::of(json!({"kind":"rebase","d":{"watcher_observed_head":head}}));
        assert!(check_review_checkout(&j,&head).is_ok());
        assert!(check_review_checkout(&j,&other).is_err());
        j.d["watcher_observed_head"]=Value::Null;
        assert!(check_review_checkout(&j,&head).is_err());
        j.d=json!({});
        assert!(check_review_checkout(&j,&other).is_ok());
    }

    #[test]
    fn zero_repo_pr_limits_mean_unlimited() {
        assert!(!repo_limit_reached(178, 20, 0, 0));
        assert!(!repo_limit_reached(178, 20, 0, 25));
        assert!(!repo_limit_reached(178, 20, 180, 0));
    }

    #[test]
    fn positive_repo_pr_limits_still_apply() {
        assert!(repo_limit_reached(180, 0, 180, 0));
        assert!(repo_limit_reached(0, 1, 0, 1));
        assert!(!repo_limit_reached(179, 0, 180, 1));
    }

    #[test]
    fn host_retry_backoff_is_bounded_and_manual_priority_preserves_phase_order() {
        assert_eq!(host_retry_delay(1),30_000);
        assert_eq!(host_retry_delay(2),60_000);
        assert_eq!(host_retry_delay(5),480_000);
        assert_eq!(host_retry_delay(20),480_000);
        let fix=json!({"st":"ready","kind":"fix","manual_priority":0,"d":{"issue_priority":3}});
        let other=json!({"st":"ready","kind":"fix","d":{"issue_priority":1}});
        let review=json!({"st":"ready","kind":"review"});
        assert!(pick_rank(&fix, 0)<pick_rank(&other, 0));
        assert!(pick_rank(&review, 0)<pick_rank(&fix, 0));
    }

    #[test]
    fn safe_defer_preserves_verified_checkpoint_and_approval() {
        let mut job=Job::of(json!({"id":1,"repo":"o/r","kind":"fix","st":"ci","d":{"h":"local","br":"fix","f0":{},"proof":{"b_exit":0,"fix_commit":"a".repeat(40)}}}));
        assert!(verified_checkpoint(&job));
        assert_eq!(deferred_delay(&mut job,false,false,false),30_000);
        assert_eq!(job.d["rs"],true);
        assert_eq!(job.d["deferred_checkpoint"],"verified");
        assert_eq!(deferred_delay(&mut job,false,true,false),60_000);
        job.d["ap"]=json!(true);
        job.st="gate".into();
        assert_eq!(deferred_delay(&mut job,true,false,false),120_000);
        assert_eq!(job.st,"submit");
        assert_eq!(job.d["ap"],true);
        assert_eq!(job.d["deferred_checkpoint"],"submit");
    }

    #[test]
    fn deferred_checkout_blocks_aliases_but_not_other_projects() {
        let db=crate::db::Db::mem();
        db.run("insert into hs(id,ssh,dk,ln,on_) values('one','same',1,2,1),('two','same',1,2,1),('other','other',1,2,1)",pa![]);
        let a=std::sync::Arc::new(crate::app::App::new(db,std::env::temp_dir(),1,"apollo-office".into()));
        let pending=a.db.jnew("o/r",1,"fix","waiting","","ready","",json!({"h":"one","rs":true,"deferred_checkpoint":"verified"}),None,0.0);
        assert!(deferred_checkout_available(&a,"o/r","two",pending));
        assert!(!deferred_checkout_available(&a,"o/r","two",pending+1));
        assert!(deferred_checkout_available(&a,"o/r","other",pending+1));
        assert!(deferred_checkout_available(&a,"o/s","two",pending+1));
    }

    #[test]
    fn recovered_fix_in_ask_mode_needs_owner_approval() {
        let db=crate::db::Db::mem();
        db.run("insert into rp(full,mode) values('o/r','ask')",pa![]);
        let a=std::sync::Arc::new(crate::app::App::new(db,std::env::temp_dir(),1,"apollo-office".into()));
        let mut j=Job::of(json!({"repo":"o/r","kind":"fix","st":"submit","d":{"rs":true}}));
        assert!(needs_owner_approval(&a,&j));
        j.d["ap"]=json!(true);
        assert!(!needs_owner_approval(&a,&j));
        j.d["ap"]=json!(false);
        a.db.run("update rp set mode='auto' where full='o/r'",pa![]);
        assert!(!needs_owner_approval(&a,&j));
    }

    #[test]
    fn hunter_requires_complete_evidence_not_just_a_title() {
        assert!(hunter_findings(&json!({"finds":[]})).unwrap().is_empty());
        assert!(hunter_findings(&Value::Null).is_err());
        assert!(hunter_findings(&json!({"finds":[{"title":"Bug","where":"src/x.rs","desc":"Reproduce"}]})).is_err());
        let valid=json!({"title":"Bug","where":"src/x.rs","desc":"Reproduce","proof":"test failure"});
        assert_eq!(hunter_findings(&json!({"finds":[valid.clone()]})).unwrap().len(),1);
        assert!(hunter_findings(&json!({"finds":[valid.clone(),valid]})).is_err());
    }

    #[test]
    fn aging_promotes_automatic_fixes_only_and_never_crosses_critical_priority() {
        let at=1+4*86_400_000;
        let mut row=json!({"kind":"fix","st":"ready","t0":1,"d":{"issue_priority":3}});
        assert_eq!(pick_rank(&row,1).2,3);
        assert_eq!(pick_rank(&row,1+86_400_000).2,2);
        assert_eq!(pick_rank(&row,at).2,1);
        row["manual_priority"]=json!(3);
        assert_eq!(pick_rank(&row,at).2,3);
        row["manual_priority"]=Value::Null;
        row["d"]["issue_priority"]=json!(0);
        assert_eq!(pick_rank(&row,at).2,0);
        row["d"]["issue_priority"]=json!(3);
        row["kind"]=json!("hunt");
        assert_eq!(pick_rank(&row,at).2,3);
    }

    #[test]
    fn ready_fixes_precede_new_hunts_and_rebases() {
        let fix = json!({"id":1,"kind":"fix","st":"ready","prio":-0.5,"d":{}});
        let hunt = json!({"id":2,"kind":"hunt","st":"ready","prio":99.0,"d":{}});
        let rebase = json!({"id":3,"kind":"rebase","st":"ready","prio":-1.0,"d":{}});
        let submit = json!({"id":4,"kind":"fix","st":"submit","prio":0.0,"d":{}});
        assert!(pick_rank(&fix, 0) < pick_rank(&hunt, 0));
        assert!(pick_rank(&fix, 0) < pick_rank(&rebase, 0));
        assert!(pick_rank(&submit, 0) < pick_rank(&fix, 0));
    }

    #[cfg(unix)]
    fn run_preflight(
        nvmrc: Option<&str>,
        node_body: &str,
        nvm_resolved: &str,
    ) -> std::process::Output {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "apollo-preflight-{}-{}",
            std::process::id(),
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let bin = root.join("bin");
        let nvm_dir = root.join("nvm");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&nvm_dir).unwrap();
        if let Some(requirement) = nvmrc {
            std::fs::write(root.join(".nvmrc"), requirement).unwrap();
        }
        for (name, body) in [
            ("git", "exit 0"),
            ("node", node_body),
            ("nvm", "printf '%s\n' \"$NVM_RESOLVED\""),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        let output = std::process::Command::new("/bin/bash")
            .arg("-c")
            .arg(ENV_PREFLIGHT)
            .current_dir(&root)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("NVM_DIR", &nvm_dir)
            .env("NVM_RESOLVED", nvm_resolved)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(root);
        output
    }

    #[cfg(unix)]
    #[test]
    fn environment_preflight_resolves_node_requirements_without_breaking_other_repositories() {
        let no_nvmrc = run_preflight(None, "echo broken-node >&2; exit 9", "");
        assert!(
            no_nvmrc.status.success(),
            "{}",
            String::from_utf8_lossy(&no_nvmrc.stdout)
        );

        let pinned = run_preflight(
            Some("v20.1.0 # keep the major stable\n"),
            "printf 'v20.19.0\n'",
            "",
        );
        assert!(
            pinned.status.success(),
            "{}",
            String::from_utf8_lossy(&pinned.stdout)
        );

        let alias = run_preflight(Some("lts/argon\n"), "printf 'v20.19.0\n'", "v20.20.0");
        assert!(
            alias.status.success(),
            "{}",
            String::from_utf8_lossy(&alias.stdout)
        );

        let unresolved = run_preflight(Some("lts/unknown\n"), "printf 'v20.19.0\n'", "N/A");
        assert!(!unresolved.status.success());
        assert!(
            String::from_utf8_lossy(&unresolved.stdout).contains("unsupported .nvmrc requirement")
        );

        let broken = run_preflight(Some("20\n"), "echo node failed >&2; exit 7", "");
        assert!(!broken.status.success());
        assert!(String::from_utf8_lossy(&broken.stdout).contains("node failed to run: node failed"));

        let mismatch = run_preflight(Some("24\n"), "printf 'v20.19.0\n'", "");
        assert!(!mismatch.status.success());
        assert!(String::from_utf8_lossy(&mismatch.stdout).contains("node incompatible"));
    }

    #[test]
    fn mergecheck_retry_preserves_head_sha_and_negative_ci_evidence() {
        let head = "0123456789abcdef0123456789abcdef01234567";
        let data = json!({"expected_head":head, "proof":{"fix_commit":null}});
        let kind = "mergecheck";
        let command = "node --test";
        let output = "not ok 1\n# fail 1";
        let first = ci_evidence(kind, &data, command, 1, output, false);
        let retry = ci_evidence(kind, &data, command, 1, output, true);
        assert_eq!(first["candidate"], json!(head));
        assert_eq!(retry["candidate"], json!(head));
        assert_eq!(retry["exit"], 1);
        assert_eq!(retry["command"], command);
        assert_eq!(retry["rerun"], true);
        let tail = retry["output_tail"].as_str().unwrap();
        assert!(tail.contains("# fail 1"));
    }

    #[test]
    fn dropped_ci_records_exit_command_candidate_and_redacted_output() {
        let head = "0123456789abcdef0123456789abcdef01234567";
        let data = json!({"ci_evidence":{
            "command":"cargo test",
            "exit":7,
            "candidate":head,
            "rerun":true,
            "output_tail":"failed with token=secret-value"
        }});
        let failure = failure_details("ci", "builder-1", &data, "CI failed: authorization=Bearer hidden", 1234);
        assert_eq!(failure["stage"], "ci");
        assert_eq!(failure["exit_code"], 7);
        assert_eq!(failure["command"], "cargo test");
        assert_eq!(failure["candidate"], head);
        assert_eq!(failure["rerun"], true);
        assert_eq!(failure["host"], "builder-1");
        assert!(failure["message"].as_str().unwrap().contains("[credencial ocultada]"));
        assert!(failure["output"].as_str().unwrap().contains("[credencial ocultada]"));
        assert!(!failure.to_string().contains("secret-value"));
        assert!(!failure.to_string().contains("hidden"));
    }

    #[test]
    fn own_fix_ci_evidence_keeps_its_fix_commit_and_green_retry_sha() {
        let fix = "fedcba9876543210fedcba9876543210fedcba98";
        let data = json!({"expected_head":null,"proof":{"fix_commit":fix}});
        let kind = "fix";
        let first = ci_evidence(kind, &data, "", 1, "", false);
        let retry = ci_evidence(kind, &data, "", 0, "", true);
        assert_eq!(first["candidate"], json!(fix));
        assert_eq!(retry["candidate"], json!(fix));
        assert_eq!(retry["exit"], 0);
        assert_eq!(retry["rerun"], true);
        assert!(first.get("rerun").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_ci_releases_build_capacity() {
        let root = std::env::temp_dir().join(format!(
            "apollo-ci-cancel-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        std::fs::create_dir_all(root.join("work/o__r")).unwrap();
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true}));
        db.run(
            "insert into hs(id,on_,ci) values('local',1,1)",
            crate::pa![],
        );
        db.run(
            "insert into rp(full,ci) values('o/r','sleep 30')",
            crate::pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "test",
            "",
            "ci",
            "",
            json!({"h":"local"}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(db, root.clone(), 1, "x".into()));
        let mut job = a.db.jg(id).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), ci(&a, &mut job))
                .await
                .is_err()
        );
        assert_eq!(a.sem.lock().unwrap().get("local").copied(), Some(0));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn setup_transport_errors_are_not_inferred_from_script_stdout() {
        let lost_worker = Out {
            c: 125,
            o: String::new(),
            e: "docker: No such container".into(),
        };
        let failed_script = Out {
            c: 1,
            o: "test reported No such container".into(),
            e: String::new(),
        };
        let cannot_spawn = Out {
            c: -1,
            o: String::new(),
            e: "cannot spawn worker process".into(),
        };
        let ssh_reset = Out {
            c: 255,
            o: String::new(),
            e: "ssh: Connection reset by peer".into(),
        };
        let stdout_only = Out {
            c: 1,
            o: "Connection reset by peer".into(),
            e: String::new(),
        };
        assert!(setup_transport_error(&lost_worker));
        assert!(!setup_transport_error(&failed_script));
        assert!(setup_transport_error(&cannot_spawn));
        assert!(setup_transport_error(&ssh_reset));
        assert!(!setup_transport_error(&stdout_only));
    }

    #[test]
    fn preflight_host_failures_are_not_missing_tool_failures() {
        let unreachable = Out {
            c: 255,
            o: String::new(),
            e: "ssh: connect to host failed".into(),
        };
        let missing = Out {
            c: 127,
            o: String::new(),
            e: "Falta: node".into(),
        };
        let absent = Out {
            c: -1,
            o: String::new(),
            e: "machine local is gone".into(),
        };
        assert!(preflight_host_error(&unreachable));
        assert!(preflight_host_error(&absent));
        assert!(!preflight_host_error(&missing));
    }

    #[tokio::test]
    async fn a_blocked_hunt_cannot_be_admitted_and_active_hunts_do_not_overlap() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,hunt,ci) values('o/r',1,'true')", pa![]);
        let first = db.jnew(
            "o/r",
            0,
            "hunt",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let second = db.jnew(
            "o/r",
            0,
            "hunt",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let fix = db.jnew(
            "o/r",
            1,
            "fix",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        db.run(
            "insert into hs(id,ln,dk,on_) values('worker1',1,0,1),('worker2',1,0,1)",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-admit-hunt-tests"),
            1,
            "apollo-office".into(),
        ));
        assert!(admit(&a, "o/r", "worker1", first));
        assert!(!admit(&a, "o/r", "worker2", second));
        // Regression: a fix must not touch the checkout of an active Hunter.
        assert!(!admit(&a, "o/r", "worker2", fix));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        // Preserve the reverse-order protection (Hunter after an active fix).
        assert!(admit(&a, "o/r", "worker1", fix));
        assert!(!admit(&a, "o/r", "worker2", first));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        for _ in 0..3 {
            crate::hunt_health::fail(&a, "o/r", "setup", 1);
        }
        assert!(!admit(&a, "o/r", "worker1", first));
        assert!(admit(&a, "o/r", "worker1", fix));
        assert_eq!(a.db.jg(first).unwrap().st, "ready");
    }

    #[tokio::test]
    async fn unblocked_ready_hunt_cannot_exceed_the_active_hunter_cap() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,hunt,ci) values('o/blocked',1,'true'),('o/healthy',1,'true')", pa![]);
        db.ks("set", &json!({"hunters": 1}));
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        let blocked_id = db.jnew(
            "o/blocked",
            0,
            "hunt",
            "blocked",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let healthy_id = db.jnew(
            "o/healthy",
            0,
            "hunt",
            "healthy",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-hunt-cap-admission-test"),
            1,
            "apollo-office".into(),
        ));
        for _ in 0..3 {
            crate::hunt_health::fail(&a, "o/blocked", "setup", 1);
        }
        assert!(!admit(&a, "o/blocked", "local", blocked_id));
        assert!(admit(&a, "o/healthy", "local", healthy_id));
        crate::hunt_health::reset(&a, "o/blocked").unwrap();
        assert!(!admit(&a, "o/blocked", "local", blocked_id));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        assert!(admit(&a, "o/blocked", "local", blocked_id));
        assert!(!admit(&a, "o/healthy", "local", healthy_id));
    }

    #[tokio::test]
    async fn queued_hunt_rechecks_repository_policy_at_admission() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into rp(full,hunt,ci) values('o/r',1,'true')",
            pa![],
        );
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        let id = db.jnew("o/r", 0, "hunt", "fixture", "", "ready", "", json!({}), None, 99.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::env::temp_dir(),
            1,
            "x".into(),
        ));
        for (revoke, restore) in [
            ("update rp set on_=0", "update rp set on_=1"),
            ("update rp set hunt=0", "update rp set hunt=1"),
            ("update rp set ci=' '", "update rp set ci='true'"),
        ] {
            a.db.run(revoke, pa![]);
            assert!(!admit(&a, "o/r", "local", id));
            assert!(a.lk.lock().unwrap().is_empty());
            assert!(a.act.lock().unwrap().is_empty());
            a.db.run(restore, pa![]);
        }
        assert!(admit(&a, "o/r", "local", id));
    }

    #[tokio::test]
    async fn concurrent_creation_does_not_duplicate_hunts() {
        let db = crate::db::Db::mem();
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        db.run(
            "insert into rp(full,ci,hunt,on_) values('o/r','true',1,1)",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-create-hunt-tests"),
            1,
            "apollo-office".into(),
        ));
        let a1 = a.clone();
        let a2 = a.clone();
        let first = std::thread::spawn(move || mk_hunt(&a1));
        let second = std::thread::spawn(move || mk_hunt(&a2));
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.into_iter().filter(|created| *created).count(), 1);
        assert_eq!(
            a.db.n(
                "select count(*) from jobs where kind='hunt' and repo='o/r'",
                pa![]
            ),
            1
        );
    }

    #[tokio::test]
    async fn blocked_ready_hunt_does_not_starve_a_healthy_repository() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 1}));
        db.run("insert into hs(id,ln,dk,on_) values('local',1,0,1)", pa![]);
        db.run(
            "insert into rp(full,ci,hunt,on_) values('o/blocked','true',1,1),('o/healthy','true',1,1)",
            pa![],
        );
        db.jnew(
            "o/blocked",
            0,
            "hunt",
            "waiting",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-blocked-hunt-capacity-test"),
            1,
            "apollo-office".into(),
        ));
        for _ in 0..3 {
            crate::hunt_health::fail(&a, "o/blocked", "setup", 1);
        }
        assert!(crate::hunt_health::blocked(&a, "o/blocked"));
        assert!(
            mk_hunt(&a),
            "a blocked ready hunt must not occupy the only lane"
        );
        assert_eq!(
            a.db.n(
                "select count(*) from jobs where kind='hunt' and repo='o/healthy'",
                pa![]
            ),
            1
        );
        assert!(!mk_hunt(&a), "the healthy hunt must still use its lane");
        crate::hunt_health::success(&a, "o/blocked");
        assert!(!mk_hunt(&a), "unblocked hunts count toward capacity again");
    }

    #[tokio::test]
    async fn policy_ineligible_ready_hunts_do_not_starve_other_repositories() {
        for (revoke, restore) in [
            ("update rp set hunt=0 where full='o/blocked'", "update rp set hunt=1 where full='o/blocked'"),
            ("update rp set on_=0 where full='o/blocked'", "update rp set on_=1 where full='o/blocked'"),
            ("update rp set ci=' ' where full='o/blocked'", "update rp set ci='true' where full='o/blocked'"),
        ] {
            let db = crate::db::Db::mem();
            db.ks("set", &json!({"hunters": 1}));
            db.run("insert into hs(id,ln,dk,on_) values('local',1,0,1)", pa![]);
            db.run(
                "insert into rp(full,ci,hunt,on_) values('o/blocked','true',1,1),('o/healthy','true',1,1)",
                pa![],
            );
            let a = std::sync::Arc::new(crate::app::App::new(
                db,
                std::env::temp_dir(),
                1,
                "apollo-office".into(),
            ));
            let id = a.db.jnew(
                "o/blocked",
                0,
                "hunt",
                "waiting",
                "",
                "ready",
                "",
                json!({}),
                None,
                99.0,
            );
            assert!(!mk_hunt(&a));
            a.db.run(revoke, pa![]);
            // A reserved lane still occupies capacity, even after revocation.
            a.lk.lock().unwrap().insert(id);
            assert!(!mk_hunt(&a));
            a.lk.lock().unwrap().remove(&id);
            assert!(mk_hunt(&a));
            assert!(!mk_hunt(&a));
            assert_eq!(
                a.db.n("select count(*) from jobs where kind='hunt' and repo='o/healthy'", pa![]),
                1
            );
            assert_eq!(
                a.db.n("select count(*) from jobs where kind='hunt' and repo='o/blocked' and st='ready'", pa![]),
                1
            );
            a.db.run(restore, pa![]);
            assert!(!mk_hunt(&a));
        }
    }

    #[test]
    fn hunter_revocation_is_terminal_but_database_failure_is_retryable() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,hunt,ci) values('o/r',1,'true')", pa![]);
        db.run(
            "insert into jobs(id,repo,kind,st) values(100,'o/r','hunt','hunt')",
            pa![],
        );
        let data = json!({"title":"Bug","body":"reproduction","hunt":{"where":"src/lib.rs","proof":"failed test"}});
        assert!(hunter_finding_result(db.queue_hunter_find("o/r", 100, &data)).unwrap());
        assert!(!hunter_finding_result(db.queue_hunter_find("o/r", 100, &data)).unwrap());
        db.run("update rp set hunt=0 where full='o/r'", pa![]);
        assert!(matches!(
            hunter_finding_result(db.queue_hunter_find("o/r", 100, &data)),
            Err(X::Drop(_))
        ));
        assert_eq!(
            db.n("select count(*) from jobs where kind='fix'", pa![]),
            1
        );
        db.run("drop table rp", pa![]);
        assert!(matches!(
            hunter_finding_result(db.queue_hunter_find("o/r", 100, &data)),
            Err(X::Defer(_))
        ));
    }

    #[tokio::test]
    async fn idle_cache_cleanup_blocks_admission_and_releases_on_drop() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
        db.run(
            "insert into hs(id,ln,dk,on_) values('worker1',1,1,1)",
            pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-idle-npm-guard-test"),
            1,
            "apollo-office".into(),
        ));
        a.lv.lock().unwrap().o.man = true;
        assert!(!admit(&a, "o/r", "worker1", id));
        assert!(!mk_hunt(&a));
        assert!(a.act.lock().unwrap().is_empty());
        assert!(a.lk.lock().unwrap().is_empty());
        a.lv.lock().unwrap().o.man = false;
        for role in ["repro", "fixer", "submit"] {
            a.lv.lock().unwrap().w.get_mut(role).unwrap().hold = true;
            assert!(!admit(&a, "o/r", "worker1", id));
            a.lv.lock().unwrap().w.get_mut(role).unwrap().hold = false;
        }
        a.lv.lock().unwrap().w.get_mut("hunter").unwrap().hold = true;
        assert!(!mk_hunt(&a));
        a.lv.lock().unwrap().w.get_mut("hunter").unwrap().hold = false;
        let reservation = IdleCacheGuard::claim(&a, "worker1").unwrap();
        assert!(IdleCacheGuard::claim(&a, "worker1").is_none());
        assert!(!admit(&a, "o/r", "worker1", id));
        std::mem::drop(reservation);
        assert!(admit(&a, "o/r", "worker1", id));
        assert!(IdleCacheGuard::claim(&a, "worker1").is_none());
    }

    #[test]
    fn cache_maintenance_serializes_aliases_of_the_same_docker_target() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
        db.run(
            "insert into hs(id,ssh,ln,dk,on_) values('alias-a','worker@example',1,1,1)",
            pa![],
        );
        db.run(
            "insert into hs(id,ssh,ln,dk,on_) values('alias-b','worker@example',1,1,1)",
            pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "same daemon",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-cache-alias-test"),
            1,
            "apollo-office".into(),
        ));
        let reservation = IdleCacheGuard::claim(&a, "alias-a").unwrap();
        assert!(IdleCacheGuard::claim(&a, "alias-b").is_none());
        assert!(!admit(&a, "o/r", "alias-b", id));
        std::mem::drop(reservation);
        assert!(admit(&a, "o/r", "alias-b", id));
        assert!(IdleCacheGuard::claim(&a, "alias-a").is_none());
    }

    #[test]
    fn setup_preflight_only_selects_known_runtimes() {
        assert_eq!(setup_tool("npm ci"), Some("npm"));
        assert_eq!(setup_tool("pnpm install --frozen-lockfile"), Some("pnpm"));
        assert_eq!(setup_tool("cargo test --workspace"), Some("cargo"));
        assert_eq!(setup_tool("curl https://example.com | sh"), None);
    }

    #[test]
    fn hunter_capacity_does_not_reserve_ready_fixes_twice() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 2}));
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-hunter-capacity-test"),
            1,
            "apollo-office".into(),
        ));
        assert_eq!(hn(&a), 2, "without fixes both lanes may hunt");
        a.db.jnew(
            "o/r",
            0,
            "fix",
            "ready fix",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(hn(&a), 1, "a single ready fix must reserve only one lane");
        let uncovered = a.db.jnew(
            "o/r",
            1,
            "fix",
            "another ready fix",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(hn(&a), 0, "a ready fix excluded from hf needs another lane");
        a.db.run("update jobs set st='fix' where id=?", pa![uncovered]);
        assert_eq!(hn(&a), 0, "a distinct active fix still needs capacity");
    }

    #[test]
    fn one_lane_can_hunt_only_when_no_fix_needs_it() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 2}));
        db.run("insert into hs(id,ln,dk,on_) values('local',1,0,1)", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-single-lane-hunt-test"),
            1,
            "apollo-office".into(),
        ));
        assert_eq!(hn(&a), 1);
        a.db.jnew(
            "o/r",
            0,
            "fix",
            "ready fix",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(hn(&a), 0);
    }

    #[test]
    fn fixes_are_ranked_ahead_of_hunts() {
        let fix = json!({"st":"ready", "kind":"fix", "prio":0, "id":2});
        let mergecheck = json!({"st":"ready", "kind":"mergecheck", "prio":0, "id":3});
        let hunt = json!({"st":"ready", "kind":"hunt", "prio":0, "id":1});
        assert!(pick_rank(&mergecheck, 0) < pick_rank(&fix, 0));
        assert!(pick_rank(&fix, 0) < pick_rank(&hunt, 0));
        let high_priority_number = json!({"st":"ready", "kind":"fix", "prio":100_000, "id":3});
        assert!(pick_rank(&high_priority_number, 0) < pick_rank(&hunt, 0));
    }

    #[test]
    fn pr_rechecks_reserve_a_worker_lane_from_hunts() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 2}));
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        db.run(
            "insert into rp(full,on_) values('o/enabled',1),('o/disabled',0)",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-recheck-capacity-test"),
            1,
            "apollo-office".into(),
        ));
        assert_eq!(hn(&a), 2);
        a.db.jnew(
            "o/enabled",
            1,
            "mergecheck",
            "Revalidate PR",
            "",
            "ready",
            "",
            json!({"expected_head":"abc"}),
            Some(42),
            -5.0,
        );
        assert_eq!(hn(&a), 1);
        a.db.jnew(
            "o/disabled",
            1,
            "mergecheck",
            "Revalidate PR",
            "",
            "ready",
            "aguarda worker",
            json!({"expected_head":"def"}),
            Some(43),
            -5.0,
        );
        assert_eq!(
            hn(&a),
            1,
            "a disabled repository must not reserve another lane"
        );
        a.db.run("update jobs set st='review' where kind='mergecheck'", pa![]);
        assert_eq!(
            hn(&a),
            1,
            "an enabled running recheck keeps its lane reservation"
        );
        a.db.run(
            "update jobs set st='done' where repo='o/enabled' and kind='mergecheck'",
            pa![],
        );
        assert_eq!(hn(&a), 2, "a disabled recheck must not suppress hunting");
    }

    #[test]
    fn optimization_verification_requires_every_named_check_before_and_after() {
        let checks = vec![
            "alpha::keeps_order".to_string(),
            "beta::uses_cache".to_string(),
        ];
        let good = "test alpha::keeps_order ... ok\ntest beta::uses_cache ... ok";
        assert!(judgeopt(&checks, 0, good, 0, good).is_empty());

        let before_missing = judgeopt(&checks, 0, "test alpha::keeps_order ... ok", 0, good);
        assert!(before_missing.contains("before the optimization"));
        assert!(before_missing.contains("beta::uses_cache"));

        let after_missing = judgeopt(&checks, 0, good, 0, "test beta::uses_cache ... ok");
        assert!(after_missing.contains("after the optimization"));
        assert!(after_missing.contains("alpha::keeps_order"));

        let red_baseline = judgeopt(
            &checks,
            0,
            "test alpha::keeps_order ... ok\ntest beta::uses_cache ... FAILED",
            0,
            good,
        );
        assert!(red_baseline.contains("before the optimization"));

        assert_eq!(
            missing_named_checks(&["test_foo".to_string()], "test test_foo_extra ... ok"),
            vec!["test_foo"]
        );
        assert_eq!(
            missing_named_checks(&["test_foo".to_string()], "test test_foo ... ignored"),
            vec!["test_foo"]
        );
        assert_eq!(
            missing_named_checks(
                &["beta::keeps_order".to_string()],
                "test alpha::keeps_order ... ok"
            ),
            vec!["beta::keeps_order"]
        );
        assert_eq!(
            missing_named_checks(
                &["alpha::keeps_order".to_string()],
                "test beta::alpha::keeps_order ... ok"
            ),
            vec!["alpha::keeps_order"]
        );
        assert!(missing_named_checks(
            &["alpha::keeps_order".to_string()],
            "test alpha::keeps_order ... ok"
        )
        .is_empty());
        assert!(
            missing_named_checks(&["rejects an error".to_string()], "✓ rejects an error")
                .is_empty()
        );
        assert!(missing_named_checks(
            &["cache_equivalence".to_string()],
            "cache_equivalence: PASS"
        )
        .is_empty());
        assert_eq!(
            missing_named_checks(
                &["rejects an error".to_string()],
                "test rejects an error ... FAILED"
            ),
            vec!["rejects an error"]
        );
        let long_log = format!(
            "test alpha::keeps_order ... ok\n{}",
            "diagnostic output ".repeat(4_000)
        );
        let tail = crate::sh::last(&long_log, 30_000);
        assert!(!missing_named_checks(&checks[..1], &tail).is_empty());
        let retained = format!("test alpha::keeps_order ... ok\n{tail}");
        assert!(missing_named_checks(&checks[..1], &retained).is_empty());
    }

    #[test]
    fn names() {
        let o = "test a::b::c ... FAILED\ntest x ... ok\nFAILED tests/t.py::test_x - assert 1 == 2\n--- FAIL: TestFoo (0.00s)\n  ✕ adds numbers (3 ms)\nnot ok 2 - parses empty\n";
        assert_eq!(
            fails(o),
            vec![
                "TestFoo",
                "a::b::c",
                "adds numbers",
                "parses empty",
                "tests/t.py::test_x"
            ]
        );
    }

    #[test]
    fn auto_merge_only_applies_to_new_bug_fix_prs() {
        let bug = Job::of(json!({"kind":"fix","d":{"feat":false}}));
        assert!(auto_merge_requested(None, true, &bug, true));
        assert!(!auto_merge_requested(None, true, &bug, false));
        assert!(!auto_merge_requested(None, false, &bug, true));
        assert!(!auto_merge_requested(Some(&json!({})), false, &bug, true));
        assert!(auto_merge_requested(Some(&json!({"auto_merge":true})), false, &bug, false));

        let feature = Job::of(json!({"kind":"fix","d":{"feat":true}}));
        assert!(!auto_merge_requested(None, true, &feature, true));
    }

    #[test]
    fn submitted_pr_preserves_latest_watcher_observation_and_pending_reply() {
        let db = crate::db::Db::mem();
        let head = "a".repeat(40);
        let published = "b".repeat(40);
        let j = Job::of(json!({"id":7,"repo":"o/r","kind":"fix","d":{}}));
        db.run("insert into rp(full) values('o/r')", pa![]);
        db.run("insert into prs(repo,num,st,upd,d) values('o/r',1,'open','fresh',?)",pa![json!({"watcher_head":head,"comment_seen_at":"fresh","seen":true,"auto_merge":false,"head_sha":head}).to_string()]);
        db.ack_pr_comments_with_reply("o/r",1,"fresh",&head,&["r1".into()],&json!({"author":"owner","body":"Thanks"})).unwrap();
        let publication = json!({"job":7,"test_cmd":"cargo test","tests":["regression"],"branch":"fix","head":"o/r","head_sha":published,"base_branch":"main","watcher_head":"stale","comment_seen_at":"old","auto_merge":true});
        record_submitted_pr(&db,&j,1,"Title","https://example.test",false,publication.clone()).unwrap();
        let row = db.one("select * from prs where repo='o/r' and num=1",pa![]).unwrap();
        assert_eq!(row["upd"],"fresh");
        assert_eq!(row["d"]["watcher_head"],head);
        assert_eq!(row["d"]["comment_seen_at"],"fresh");
        assert_eq!(row["d"]["head_sha"],published);
        assert_eq!(row["d"]["auto_merge"],false);
        assert_eq!(db.watcher_outbox_counts().unwrap()["pending"],1);
        db.run("update prs set st='merged' where repo='o/r' and num=1",pa![]);
        assert!(record_submitted_pr(&db,&j,1,"Title","https://example.test",false,publication).is_err());
        assert_eq!(db.one("select st from prs where num=1",pa![]).unwrap()["st"],"merged");
    }

    #[test]
    fn submitted_pr_rejects_corrupt_metadata_or_revoked_repository() {
        let db = crate::db::Db::mem();
        let j = Job::of(json!({"id":7,"repo":"o/r","kind":"fix","d":{}}));
        let publication = json!({"job":7,"test_cmd":"cargo test","tests":["regression"],"branch":"fix","head":"o/r","head_sha":"a".repeat(40),"base_branch":"main"});
        db.run("insert into rp(full) values('o/r')",pa![]);
        record_submitted_pr(&db,&j,1,"Title","https://example.test",true,publication.clone()).unwrap();
        assert_eq!(db.one("select d from prs where num=1",pa![]).unwrap()["d"]["auto_merge"],true);
        db.run("update prs set d='invalid' where num=1",pa![]);
        assert!(record_submitted_pr(&db,&j,1,"Title","https://example.test",false,publication.clone()).is_err());
        db.run("update rp set on_=0 where full='o/r'",pa![]);
        assert!(record_submitted_pr(&db,&j,2,"Title","https://example.test",true,publication).is_err());
        assert_eq!(db.n("select count(*) from prs",pa![]),1);
    }

    fn submission_fixture() -> (crate::db::Db, Job, Value, Value) {
        let db = crate::db::Db::mem();
        let j = Job::of(json!({"id":7,"repo":"o/r","kind":"fix","d":{}}));
        db.run("insert into rp(full) values('o/r')",pa![]);
        db.run("insert into jobs(id,repo,kind,st) values(7,'o/r','fix','submit')",pa![]);
        let sha = "a".repeat(40);
        let publication = json!({"job":7,"test_cmd":"cargo test","tests":["regression"],"branch":"fix","head":"o/r","head_sha":sha,"base_branch":"main"});
        let remote = json!({"id":123,"number":12,"state":"open","html_url":"https://github.com/o/r/pull/12","base":{"repo":{"full_name":"o/r"},"ref":"main"},"head":{"repo":{"full_name":"o/r"},"ref":"fix","sha":sha}});
        (db,j,publication,remote)
    }

    #[test]
    fn publication_receipt_survives_failed_registration_without_second_creation() {
        let (db,j,publication,remote) = submission_fixture();
        let mut checkpoint = begin_submit_publication(&db,&j,"Title",&publication,None).unwrap();
        assert!(checkpoint["number"].is_null());
        assert_eq!(checkpoint["auto_merge"],true);
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
        // Simulate successful creation once, followed by failure of local registration.
        save_submit_number(&db,&j,&mut checkpoint,12).unwrap();
        db.run("create trigger fail_registration before insert on prs begin select raise(abort,'injected failure'); end",pa![]);
        assert!(persist_submit_receipt(&db,&j,&checkpoint,&remote).is_err());
        let recovered = load_submit_publication(&db,&j).unwrap().unwrap();
        assert_eq!(recovered["number"],12);
        assert_eq!(recovered["auto_merge"],true);
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
        db.run("drop trigger fail_registration",pa![]);
        assert_eq!(persist_submit_receipt(&db,&j,&recovered,&remote).unwrap(),("https://github.com/o/r/pull/12".into(),12));
        assert_eq!(db.one("select d from prs where num=12",pa![]).unwrap()["d"]["auto_merge"],true);
        let mut closed = remote.clone();
        closed["state"] = json!("closed");
        assert!(persist_submit_receipt(&db,&j,&recovered,&closed).is_err());
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
    }

    #[test]
    fn startup_preserves_auto_publication_receipt_without_granting_approval() {
        let (db,j,publication,remote) = submission_fixture();
        db.run("update rp set mode='auto' where full='o/r'",pa![]);
        let mut checkpoint = begin_submit_publication(&db,&j,"Title",&publication,None).unwrap();
        save_submit_number(&db,&j,&mut checkpoint,12).unwrap();
        let a = std::sync::Arc::new(crate::app::App::new(db,std::env::temp_dir(),1,"x".into()));
        recover(&a).unwrap();
        let resumed = a.db.jg(j.id).unwrap();
        assert_eq!(resumed.st,"submit");
        assert_ne!(resumed.d["ap"],true);
        assert!(!needs_owner_approval(&a,&resumed));
        let saved = load_submit_publication(&a.db,&resumed).unwrap().unwrap();
        assert_eq!(saved,checkpoint);
        assert!(begin_submit_publication(&a.db,&resumed,"Title",&publication,None).is_err());
        assert_eq!(persist_submit_receipt(&a.db,&resumed,&saved,&remote).unwrap().1,12);
        assert_eq!(a.db.n("select count(*) from prs",pa![]),1);
        a.db.run("update rp set mode='manual' where full='o/r'",pa![]);
        recover(&a).unwrap();
        let resumed = a.db.jg(j.id).unwrap();
        assert_eq!(resumed.st,"submit");
        assert!(needs_owner_approval(&a,&resumed));
        assert_ne!(resumed.d["ap"],true);
        assert_eq!(load_submit_publication(&a.db,&resumed).unwrap().unwrap(),checkpoint);
    }

    #[test]
    fn publication_intent_rechecks_cancellation_inside_transaction() {
        let (db,j,publication,_) = submission_fixture();
        assert!(!cancellation_requested(&db,j.id).unwrap());
        db.ks(&format!("kill:{}",j.id),&json!(true));
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
        assert!(load_submit_publication(&db,&j).unwrap().is_none());
        db.ks(&format!("kill:{}",j.id),&json!("invalid"));
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
        db.ks(&format!("kill:{}",j.id),&json!(false));
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_ok());
    }

    #[test]
    fn proven_unstarted_intent_can_retry_but_confirmed_receipt_cannot_be_cleared() {
        let (db,j,publication,remote) = submission_fixture();
        let first = begin_submit_publication(&db,&j,"Title",&publication,None).unwrap();
        release_unstarted_publication(&db,&j,&first).unwrap();
        assert!(load_submit_publication(&db,&j).unwrap().is_none());
        let mut second = begin_submit_publication(&db,&j,"Title",&publication,None).unwrap();
        save_submit_number(&db,&j,&mut second,12).unwrap();
        assert!(release_unstarted_publication(&db,&j,&first).is_err());
        assert!(release_unstarted_publication(&db,&j,&second).is_err());
        assert_eq!(persist_submit_receipt(&db,&j,&second,&remote).unwrap().1,12);
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
    }

    #[test]
    fn ambiguous_publication_intent_never_allows_another_creation() {
        let (db,j,publication,remote) = submission_fixture();
        begin_submit_publication(&db,&j,"Title",&publication,None).unwrap();
        let recovered = load_submit_publication(&db,&j).unwrap().unwrap();
        assert!(persist_submit_receipt(&db,&j,&recovered,&remote).is_err());
        assert!(begin_submit_publication(&db,&j,"Title",&publication,None).is_err());
        assert_eq!(db.n("select count(*) from prs",pa![]),0);
    }

    #[test]
    fn submitted_pr_inventory_rejects_ambiguous_and_incomplete_targets() {
        let (_,_,publication,remote) = submission_fixture();
        assert_eq!(select_submit_pr(&[],"o/r",&publication).unwrap(),None);
        assert_eq!(select_submit_pr(std::slice::from_ref(&remote),"o/r",&publication).unwrap(),Some(12));
        assert!(select_submit_pr(&[json!({})],"o/r",&publication).is_err());
        assert!(select_submit_pr(&[remote.clone(),remote.clone()],"o/r",&publication).is_err());
        for (field,value) in [("sha",json!("b".repeat(40))),("repo",Value::Null),("repo",json!({"full_name":""}))] {
            let mut changed=remote.clone();
            changed["head"][field]=value;
            assert!(select_submit_pr(&[changed],"o/r",&publication).is_err());
        }
        let mut another=remote.clone();
        another["number"]=json!(13);
        another["html_url"]=json!("https://github.com/o/r/pull/13");
        assert!(select_submit_pr(&[remote,another],"o/r",&publication).is_err());
    }

    #[test]
    fn infra() {
        let o = "CI: 6 passed, FAILED at `deny`: cargo-deny is required in CI: cargo install cargo-deny --locked\nxtask ci: error: ci failed at `deny`\n[exit 1]";
        assert!(!lost(o));
        assert!(lost("Compiling foo\n"));
        assert_eq!(tool(o).unwrap().1.as_deref(), Some("cargo-deny"));
        assert_eq!(
            tool("error: no such command: `nextest`\n[exit 101]")
                .unwrap()
                .1
                .as_deref(),
            Some("cargo-nextest")
        );
        assert_eq!(
            tool("sh: 1: pnpm: command not found\n[exit 127]")
                .unwrap()
                .1,
            None
        );
        assert!(tool("test a ... FAILED\n[exit 101]").is_none());
    }

    #[test]
    fn judging() {
        let ts = vec!["m::t_empty".to_string()];
        assert!(judge(
            &ts,
            101,
            "test m::t_empty ... FAILED\npanicked at x",
            0,
            "test m::t_empty ... ok"
        )
        .is_empty());
        assert!(judge(&ts, 0, "", 0, "").contains("already passes"));
        assert!(judge(&ts, 101, "error[E0425]: cannot find", 0, "").contains("does not compile"));
        assert!(judge(&ts, 1, "something else failed", 0, "").contains("No failure"));
        assert!(
            judge(&ts, 101, "test m::t_empty ... FAILED", 1, "boom").contains("fails with the fix")
        );
        assert!(judge(&ts, 101, "t_empty FAILED", 1, "boom").contains("No failure"));
        assert!(judge(&ts, 1, "t_empty", 0, "[checkout failed]").contains("checkout"));
        assert!(judgef(
            &ts,
            101,
            "error[E0425]: cannot find function",
            0,
            "test m::t_empty ... ok"
        )
        .is_empty());
        assert!(judgef(
            &ts,
            101,
            "error[E0425]: cannot find function",
            0,
            "test other ... ok"
        )
        .contains("None of the named"));
        assert!(judgef(
            &ts,
            101,
            "error[E0425]: cannot find function",
            1,
            "test m::t_empty ... FAILED"
        )
        .contains("fails with the feature"));
        assert!(judgef(&ts, 0, "", 0, "").contains("already passes"));
        assert!(
            judgef(&ts, 101, "test m::t_empty ... FAILED", 1, "boom").contains("with the feature")
        );
    }

    #[test]
    fn optimization_requires_the_same_named_checks_to_pass_before_and_after() {
        let ts = vec!["suite::case".to_string()];
        let pass = "test suite::case ... ok";
        assert!(judgeopt(&ts, 0, pass, 0, pass).is_empty());
        assert!(judgeopt(&ts, 101, "test suite::case ... FAILED", 0, pass)
            .contains("fail before the optimization"));
        assert!(judgeopt(&ts, 0, pass, 101, "test suite::case ... FAILED")
            .contains("fail after the optimization"));
        assert!(judgeopt(&ts, 0, "running 0 tests", 0, pass)
            .contains("Named checks are missing or did not pass before"));
        assert!(judgeopt(&ts, 0, pass, 0, "running 0 tests")
            .contains("Named checks are missing or did not pass after"));
        assert!(judgeopt(&ts, 0, "[checkout failed]", 0, pass).contains("checkout"));
    }

    #[test]
    fn pr_quota_zero_means_unlimited() {
        // A missing cap does not block either new PRs or work already in flight.
        assert!(!repo_limit_reached(0, 0, 0, 0));
        assert!(!repo_limit_reached(2, 1_000_000, 0, 0));
        // Positive caps retain the existing inclusive boundary.
        assert!(!repo_limit_reached(1, 0, 2, 0));
        assert!(repo_limit_reached(2, 0, 2, 0));
        assert!(repo_limit_reached(3, 0, 2, 0));
        assert!(!repo_limit_reached(0, 4, 0, 5));
        assert!(repo_limit_reached(0, 5, 0, 5));
    }

    #[test]
    fn conventional_commit_subject_is_distinct_from_pull_request_title() {
        let portuguese = "Rejeitar datas civis inválidas nas entregas RMA";
        let title = pr_title(None, &json!({"title": portuguese}), "outra causa").unwrap();
        let subject = commit_subject(&title, false).unwrap();
        assert_eq!(
            subject,
            "fix: Rejeitar datas civis inválidas nas entregas RMA"
        );
        // PR titles remain human-readable and do not acquire a forced prefix.
        assert_eq!(title, portuguese);
        assert_eq!(
            commit_subject("fix(rma): validar datas civis", false).unwrap(),
            "fix(rma): validar datas civis"
        );
        assert_eq!(
            commit_subject("Adicionar avisos de RMA", true).unwrap(),
            "feat: Adicionar avisos de RMA"
        );
        assert_eq!(
            commit_subject("Corrigir falha\nMSG\ncomando inesperado", false).unwrap(),
            "fix: Corrigir falha"
        );
        assert!(commit_subject("   \n ", false).is_err());
        let long = "x".repeat(200);
        assert_eq!(commit_subject(&long, false).unwrap().chars().count(), 120);
    }

    #[test]
    fn optimization_commits_do_not_claim_unmeasured_performance_gains() {
        assert_eq!(
            optimization_commit_subject("perf: Reduce repeated parsing").unwrap(),
            "refactor: Reduce repeated parsing"
        );
        assert_eq!(
            optimization_commit_subject("Simplify internal lookup").unwrap(),
            "refactor: Simplify internal lookup"
        );
        assert!(optimization_commit_subject("perf: ").is_err());
        assert!(optimization_commit_subject("refactor(core):").is_err());
    }

    #[test]
    fn pull_request_title_falls_back_when_generated_title_is_blank() {
        let description = json!({"title": "   "});
        assert_eq!(
            pr_title(None, &description, "Datas de RMA inválidas").unwrap(),
            "Datas de RMA inválidas"
        );
        assert_eq!(
            pr_title(Some("Título aprovado"), &description, "Outro").unwrap(),
            "Título aprovado"
        );
        assert_eq!(
            pr_title(None, &json!({"title": "  Linha útil\nMSG"}), "").unwrap(),
            "Linha útil"
        );
        assert!(pr_title(None, &json!({"title": ""}), "  ").is_err());
    }

    #[test]
    fn bodies() {
        let j = Job::of(
            json!({"id": 5, "repo": "o/r", "num": 12, "kind": "fix", "title": "T", "d": {"mn": "main", "known": []}}),
        );
        let f = json!({"cause": "c", "fix": "f", "tests": ["t1"], "test_cmd": "cargo test t1", "untouched": ""});
        let b = body_lang(&j, &f, "cargo test", FOOT, false);
        assert!(b.starts_with("Fixes #12"));
        assert!(b.contains("`t1`: fails on `main`"));
        assert!(b.ends_with(FOOT));
        assert!(!body_lang(&j, &f, "cargo test", "", false).contains(FOOT));
        assert!(
            body_lang(&j, &f, "cargo test", FOOT0, false).ends_with(FOOT0)
                && !FOOT0.contains("ApolloOffice")
        );
        let h = Job::of(
            json!({"id": 6, "repo": "o/r", "num": 0, "kind": "fix", "title": "T", "d": {"mn": "main", "hunt": {"where": "a.rs:3", "title": "x"}}}),
        );
        let b = body_lang(
            &h,
            &json!({"cause": "Fixes #0. c", "fix": "f", "tests": ["a", "b"], "test_cmd": "t"}),
            "make test",
            FOOT,
            false,
        );
        assert!(b.starts_with("Found while testing `a.rs:3`"));
        assert!(!b.contains("#0"));
        assert!(b.contains("Regression tests `a`, `b`: fail on"));
        let fj = Job::of(
            json!({"id": 7, "repo": "o/r", "num": 9, "kind": "fix", "title": "T", "d": {"mn": "main", "feat": true}}),
        );
        let b = body_lang(&fj, &f, "cargo test", "", false);
        assert!(b.starts_with("Closes #9"));
        assert!(b.contains("## Why") && b.contains("## Change") && !b.contains("## Cause"));
        assert!(b.contains("Test `t1`: fails on `main` without the change"));
    }

    #[test]
    fn issue_closure_survives_pr_templates_without_duplicating_links() {
        let bug = Job::of(json!({"id": 5, "repo": "o/r", "num": 12, "kind": "fix", "d": {}}));
        assert_eq!(ensure_issue_closure(&bug, "Template content".into()), "Template content\n\nFixes #12");
        assert_eq!(ensure_issue_closure(&bug, "Template content\n\ncloses #12".into()), "Template content\n\ncloses #12");
        assert_eq!(ensure_issue_closure(&bug, "`Closes #12`".into()), "`Closes #12`\n\nFixes #12");
        assert_eq!(ensure_issue_closure(&bug, "```text\nCloses #12\n```".into()), "```text\nCloses #12\n```\n\nFixes #12");

        let feature = Job::of(json!({"id": 6, "repo": "o/r", "num": 13, "kind": "fix", "d": {"feat": true}}));
        assert_eq!(ensure_issue_closure(&feature, "Template content".into()), "Template content\n\nCloses #13");

        let found_by_hunter = Job::of(json!({"id": 7, "repo": "o/r", "num": 0, "kind": "fix", "d": {"hunt": {"title": "bug"}}}));
        assert_eq!(ensure_issue_closure(&found_by_hunter, "Template content".into()), "Template content");
    }

    #[test]
    fn junks() {
        assert!(junk("__pycache__/tb.cpython-314.pyc"));
        assert!(junk("tests/__pycache__/x.pyc"));
        assert!(junk("build/a.o"));
        assert!(!junk("tb.py"));
        assert!(!junk("src/main.rs"));
    }

    #[test]
    fn gitcfg() {
        let u = "https://github.com/o/r.git";
        let l = "core.repositoryformatversion=0\ncore.filemode=true\nremote.origin.url=https://github.com/o/r.git\nremote.origin.fetch=+refs/heads/*:refs/remotes/origin/*\nremote.origin.promisor=true\nremote.origin.partialclonefilter=blob:none\nbranch.main.remote=origin\nbranch.main.merge=refs/heads/main\nuser.name=T\nuser.email=t@t\nextensions.partialclone=origin\n";
        assert!(cfg_ok(l, u).is_ok());
        assert!(cfg_ok(&format!("{l}core.fsmonitor=curl x\n"), u).is_err());
        assert!(cfg_ok(&format!("{l}core.hookspath=/tmp/h\n"), u).is_err());
        assert!(cfg_ok(
            &format!("{l}url.https://evil/.insteadof=https://github.com/\n"),
            u
        )
        .is_err());
        assert!(cfg_ok(&format!("{l}include.path=/tmp/x\n"), u).is_err());
        assert!(cfg_ok(&l.replace("github.com/o/r", "evil.com/o/r"), u).is_err());
    }

    #[test]
    fn slugs() {
        let j = Job::of(
            json!({"id": 5, "repo": "o/r", "num": 12, "title": "Crash on empty input!!", "d": {}}),
        );
        assert_eq!(bslug(&j), "fix/12-crash-on-empty-input");
        assert_eq!(slug("a/b"), "a__b");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn subscription_review_returns_free_author_feedback_and_stages_proposal() {
        let _cli_lock = crate::ai::CLI_TEST_LOCK.lock().await;
        use std::os::unix::fs::PermissionsExt;
        let dd = std::env::temp_dir().join(format!(
            "apollo-review-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("sum.py"), "def add(a,b): return a-b\n").unwrap();
        git(&["add", "sum.py"]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-qm",
            "base",
        ]);
        let base = git(&["rev-parse", "HEAD"]);
        std::fs::write(repo.join("sum.py"), "def add(a,b): return 5\n").unwrap();
        git(&["add", "sum.py"]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-qm",
            "bad hardcoded fix",
        ]);
        let cli = dd.join("review-cli");
        std::fs::write(&cli,r##"#!/usr/bin/python3
import sys,json,pathlib
args=sys.argv[1:];text=sys.stdin.read()
assert 'read-only' in args and '--ignore-user-config' in args
assert not any('mcp_servers.office' in x for x in args), 'reviewer must have no worker tools'
assert 'def add(a,b): return 5' in text
assert 'real fixture verification: negative addition fails' in text
answer=json.dumps({'ok':False,'issues':['Replace hardcoded 5 with a+b; test negative values too.'],'lesson':{'mistake':'Hardcoded example instead of general arithmetic.','correction':'Implement a+b.','verification':'Exercise positive and negative integers.'}})
pathlib.Path(args[args.index('-o')+1]).write_text(answer)
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':10,'output_tokens':10}}))
"##).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("APOLLO_OFFICE_CODEX", &cli);
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true}));
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run(
            "insert into rp(full,ci) values('o/r','python3 sum.py')",
            pa![],
        );
        let id=db.jnew("o/r",0,"fix","addition bug","", "fix","",json!({"h":"local","base":base,"ext":{"src":"fixture"},"verification_feedback":"real fixture verification: negative addition fails"}),None,0.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        let mut job = a.db.jg(id).unwrap();
        let mut f = json!({"title":"Fix addition","cause":"subtraction","fix":"hardcode five"});
        let feedback = review(&a, &mut job, &mut f).await.unwrap();
        std::env::remove_var("APOLLO_OFFICE_CODEX");
        assert!(feedback.contains("Replace hardcoded 5"));
        assert_eq!(a.db.jg(id).unwrap().d["last_review"]["model"], "codex:");
        assert_eq!(
            a.db.n("select count(*) from kv where k like 'lesson:%'", pa![]),
            1
        );
        assert!(
            !a.db.jg(id).unwrap().d["ms"]
                .as_array()
                .map(|v| v.contains(&json!("codex:")))
                .unwrap_or(false),
            "review must not claim code authorship"
        );
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[test]
    fn java_compile_or_loading_failure_is_not_a_regression_proof() {
        let names = vec!["channel_identity".to_string()];
        assert!(judge(
            &names,
            1,
            "BlockedChannelData.java:24: error: cannot find symbol\nchannel_identity",
            0,
            "channel_identity OK"
        )
        .contains("compile or load"));
        assert!(judge(
            &names,
            1,
            "channel_identity java.lang.ClassNotFoundException: BlockedChannelData",
            0,
            "channel_identity OK"
        )
        .contains("compile or load"));
        assert!(judge(
            &names,
            1,
            "channel_identity java.lang.AssertionError: distinct IDs collide",
            0,
            "channel_identity OK"
        )
        .is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn independent_acceptance_rejects_author_tests_green_and_feedback_reaches_fixer() {
        use sha2::{Digest, Sha256};
        let dd = std::env::temp_dir().join(format!(
            "apollo-independent-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let commit = || {
            git(&["add", "."]);
            git(&[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-qm",
                "fixture",
            ]);
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q", "-b", "work"]);
        std::fs::write(repo.join("maths.py"), "def add(a,b): return a-b\n").unwrap();
        let base = commit();
        std::fs::write(repo.join("author_test.py"),"from maths import add\nimport sys\nok=add(2,3)==5\nprint('positive_addition', 'PASS' if ok else 'FAIL')\nsys.exit(0 if ok else 1)\n").unwrap();
        commit();
        std::fs::write(repo.join("maths.py"), "def add(a,b): return 5\n").unwrap();
        commit();
        let script="import os,pathlib,runpy\nm=runpy.run_path(str(pathlib.Path(os.environ['APOLLO_SOURCE_ROOT'])/'maths.py'))\nok=m['add'](-2,3)==1\nprint('APOLLO_CHECK:negative_addition:'+('PASS' if ok else 'FAIL'))\nraise SystemExit(0 if ok else 1)\n";
        let frozen = json!({"base":base,"baseline_valid":true,"designer":"fixture-independent-reviewer","checks":["negative_addition"],"script":script});
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"quality_checks":true}));
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run(
            "insert into rp(full,ci) values('o/r','python3 author_test.py')",
            pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "addition",
            "",
            "fix",
            "",
            json!({"h":"local","base":base,"br":"work","acceptance":frozen}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        let mut job = a.db.jg(id).unwrap();
        let feedback=verify(&a,&mut job,&json!({"tests":["positive_addition"],"test_cmd":"PYTHONDONTWRITEBYTECODE=1 python3 author_test.py"})).await.unwrap();
        assert_eq!(
            job.d["proof"]["b_exit"], 0,
            "author's tests must really be green"
        );
        assert!(feedback.contains("negative_addition"), "{feedback}");
        assert_eq!(job.d["acceptance_proof"]["ok"], false);
        assert!(fixp(&a, &job, &feedback, "").contains("Independent acceptance rejected"));
        assert_eq!(
            job.d["acceptance_proof"]["run"]["script_sha256"],
            json!(hex::encode(Sha256::digest(script.as_bytes())))
        );
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_with_several_commits_is_squashed() {
        let dd = std::env::temp_dir().join(format!(
            "apollo-squash-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let commit = |m: &str| {
            git(&["add", "."]);
            git(&[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-qm",
                m,
            ]);
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q", "-b", "work"]);
        git(&["config", "user.name", "fixture"]);
        git(&["config", "user.email", "fixture@localhost"]);
        std::fs::write(repo.join("a.txt"), "1\n").unwrap();
        let base = commit("base");
        std::fs::write(repo.join("a.txt"), "2\n").unwrap();
        commit("Address review");
        std::fs::write(repo.join("b.txt"), "3\n").unwrap();
        commit("more");
        let db = crate::db::Db::mem();
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
        let id = db.jnew(
            "o/r",
            1,
            "review",
            "t",
            "",
            "fix",
            "",
            json!({"h":"local","base":base,"br":"work"}),
            Some(1),
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        let job = a.db.jg(id).unwrap();
        let fb = vrev(&a, &job, &json!({})).await.unwrap();
        assert_eq!(fb, "");
        assert_eq!(git(&["rev-list", "--count", &format!("{base}..HEAD")]), "1");
        assert_eq!(git(&["log", "-1", "--format=%s"]), "Address review");
        assert_eq!(git(&["show", "HEAD:b.txt"]), "3");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resumed_free_first_submission_without_owner_approval_never_invokes_github() {
        use std::os::unix::fs::PermissionsExt;
        let dd = std::env::temp_dir().join(format!(
            "apollo-publish-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(out.status.success());
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["init", "-q", "-b", "work"]);
        std::fs::write(repo.join("file"), "fixture\n").unwrap();
        git(&["add", "file"]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-qm",
            "base",
        ]);
        let base = git(&["rev-parse", "HEAD"]);
        let marker = dd.join("github-called");
        let gh = dd.join("gh");
        std::fs::write(
            &gh,
            format!(
                "#!/bin/sh\nprintf called > {}\nexit 97\n",
                q(&marker.display().to_string())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("APOLLO_OFFICE_GH", &gh);
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true}));
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run("insert into rp(full,mode) values('o/r','auto')", pa![]);
        let id=db.jnew("o/r",0,"fix","fixture","","ready","resumed",json!({"h":"local","base":base,"br":"work","rs":true,"ap":false,"f":{"title":"Fixture","tests":[],"test_cmd":"true"}}),None,0.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        // Missing and malformed grants, plus an unusable preview base, must
        // all remain held. The fake GitHub executable detects publication.
        for grant in [json!(false), Value::Null, json!("true"), json!(1)] {
            let mut job = a.db.jg(id).unwrap();
            job.d["ap"] = grant;
            job.d["base"] = json!("nonexistent-preview-base");
            a.db.jset(id, &[("st", json!("ready"))]);
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(2), submit(&a, &mut job)).await;
            assert!(
                !marker.exists(),
                "resuming a job must not invoke GitHub without owner approval"
            );
            assert!(matches!(result,Ok(Err(X::Back(ref reason))) if reason.is_empty()));
            assert_eq!(a.db.jg(id).unwrap().st, "approve");
        }
        std::env::remove_var("APOLLO_OFFICE_GH");
        std::fs::remove_dir_all(dd).unwrap();
    }
    // ---- staged RED/GREEN two-phase TDD ----

    #[cfg(unix)]
    fn staged_fixture() -> (Ax, i64, std::path::PathBuf, std::path::PathBuf) {
        let dd = std::env::temp_dir().join(format!("apollo-staged-{}", hex::encode(rand::random::<[u8; 8]>())));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(repo.join("t")).unwrap();
        let git = {
            let repo = repo.clone();
            move |args: &[&str]| {
                let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
                assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
        };
        // Intentional bug: add subtracts instead of adding.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 - $2 )); }\n").unwrap();
        std::fs::write(repo.join("t/run.sh"), "e=0\nfor f in t/*_test.sh; do bash \"$f\" || e=1; done\nexit $e\n").unwrap();
        git(&["init", "-q", "-b", "work"]);
        git(&["config", "user.name", "fixture"]);
        git(&["config", "user.email", "fixture@localhost"]);
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        let base = git(&["rev-parse", "HEAD"]);
        let db = crate::db::Db::mem();
        // Pilot config: staged on, allowlisted repo, free-first OFF (required).
        db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"]}));
        db.run("insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)", pa![]);
        // Mandatory GREEN production allowlist for the pilot repo: only add.sh.
        db.run("insert into rp(full,ci,green) values('o/r','bash t/run.sh','^add\\.sh$')", pa![]);
        let id = db.jnew("o/r", 0, "fix", "wrong addition", "", "fix", "", json!({"h":"local","base":base,"br":"work","repro":{"how":"add 2 3 prints -1"}}), None, 0.0);
        let a = std::sync::Arc::new(crate::app::App::new(db, dd.clone(), 1, "apollo-office".into()));
        (a, id, dd, repo)
    }

    #[cfg(unix)]
    const RED_FAIL: &str = "source ./add.sh\n[ \"$(add 2 3)\" = 5 ] && echo 'ok 1 - sum' || { echo 'not ok 1 - sum'; exit 1; }\n";

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_happy_path_makes_test_then_fix_commit_with_real_red_green() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let git = |args: &[&str]| String::from_utf8_lossy(&std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // RED: author writes only the failing test.
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "add returns a wrong sum").await.unwrap();
        assert!(fb.is_none(), "RED should be accepted: {fb:?}");
        assert_eq!(git(&["rev-list", "--count", &format!("{base}..HEAD")]), "1");
        assert!(git(&["diff", "--name-only", &base, "HEAD"]).lines().all(|l| l == "t/sum_test.sh"));
        assert!(git(&["log", "-1", "--format=%s"]).starts_with("test:"));
        let red_files: Vec<String> = git(&["diff", "--name-only", &base, "HEAD"]).lines().map(|l| l.to_string()).collect();
        let test_commit = git(&["rev-parse", "HEAD"]);
        // GREEN: author writes only the fix.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let js = json!({"title":"Add instead of subtract","test_cmd":"bash t/run.sh","tests":["sum"],"cause":"c","fix":"f"});
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "Add instead of subtract").await.unwrap();
        assert!(fb.is_none(), "GREEN should be accepted: {fb:?}");
        assert_eq!(git(&["rev-list", "--count", &format!("{base}..HEAD")]), "2");
        assert!(git(&["log", "-1", "--format=%s"]).starts_with("fix:"));
        assert!(git(&["diff", "--name-only", "HEAD~1", "HEAD"]).lines().all(|l| l == "add.sh"), "fix commit must not touch the test");
        // The authoritative two-commit gate agrees.
        let v = verify(&a, &mut job, &js).await.unwrap();
        assert_eq!(v, "", "verify must confirm red on the test commit and green on the fix commit");
        assert_ne!(job.d["proof"]["a_exit"], 0);
        assert_eq!(job.d["proof"]["b_exit"], 0);
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_rejects_implementation_changes() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap(); // sneaks the fix in
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("test files only"), "{fb:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{}..HEAD", job.s("base"))]).current_dir(&repo).output().unwrap().stdout).trim(), "0", "no commit on rejection");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_rejects_test_that_already_passes() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), "echo 'ok 1 - sum'\n").unwrap();
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("must fail"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_rejects_compile_or_load_error() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), "echo placeholder\n").unwrap();
        // A test that does not load (here a missing import) is not a real failing
        // test. Python's error text is locale-independent, unlike a shell's.
        let fb = red_commit(&a, &mut job, "python3 -c 'import nonexistent_mod_xyz'", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("compile or load"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_green_rejects_modifying_the_committed_test() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none());
        let red_files: Vec<String> = String::from_utf8_lossy(&std::process::Command::new("git").args(["diff", "--name-only", &base, "HEAD"]).current_dir(&repo).output().unwrap().stdout).lines().map(|l| l.to_string()).collect();
        let test_commit = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // GREEN weakens the test and adds the fix.
        std::fs::write(repo.join("t/sum_test.sh"), "echo 'ok 1 - sum'\n").unwrap();
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_green_rejects_when_test_still_fails() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none());
        let test_commit = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        std::fs::write(repo.join("add.sh"), "add() { echo 0; }\n").unwrap(); // allowlisted but not an actual fix
        let fb = green_commit(&a, &mut job, &["t/sum_test.sh".to_string()], &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("still fails"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_mode_is_opt_in_and_scoped() {
        let (a, id, dd, _repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        assert!(staged(&a, &job), "fix job with staged_tdd + allowlisted repo + free_first OFF is staged");
        job.d["feat"] = json!(true);
        assert!(!staged(&a, &job), "feature requests keep the legacy flow");
        job.d.as_object_mut().unwrap().remove("feat");
        job.kind = "review".into();
        assert!(!staged(&a, &job), "PR-review jobs keep the legacy flow");
        job.kind = "fix".into();
        // B5: another repo (e.g. FixFlow) is never staged even with every flag on.
        let mut other = job.clone();
        other.repo = "croquete1/FixFlow-novo".into();
        assert!(!staged(&a, &other) && !staged_intended(&a, &other), "non-allowlisted repo (FixFlow) must stay on the legacy flow");
        a.db.ks("set", &json!({"staged_tdd":true})); // allowlist removed
        assert!(!staged(&a, &job), "fails closed: no allowlist means no repo is staged");
        a.db.ks("set", &json!({"staged_repos":["o/r"]})); // opt-in flag off
        assert!(!staged(&a, &job), "off without the opt-in flag");
        // V3-P1-3: with free_first ON the mode is INTENDED but NOT active (it would
        // substitute the configured FIXER/REVIEWER); fix() rejects that config.
        a.db.ks("set", &json!({"free_first":true,"staged_tdd":true,"staged_repos":["o/r"]}));
        assert!(staged_intended(&a, &job), "still intended with free_first on");
        assert!(!staged(&a, &job), "but NOT active under free-first routing (fail closed)");
        a.db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"]})); // free_first OFF
        assert!(staged(&a, &job), "active with the flag, allowlist and free_first off");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[test]
    fn testish_classifies_common_layouts_and_honours_repo_override() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full) values('o/r')", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()));
        let j = crate::db::Job::of(json!({"repo":"o/r","kind":"fix"}));
        for p in ["t/sum_test.sh", "tests/test_x.py", "src/foo_test.go", "spec/foo_spec.rb", "FooTest.java", "__tests__/a.js", "conftest.py"] {
            assert!(testish(&a, &j, p), "expected test: {p}");
        }
        for p in ["src/add.sh", "lib/maths.py", "main.go", "README.md"] {
            assert!(!testish(&a, &j, p), "expected non-test: {p}");
        }
        // explicit per-repo override (rp.tests regex) wins over the heuristic
        let a2 = {
            let db = crate::db::Db::mem();
            db.run("insert into rp(full,tests) values('o/r','^impl/')", pa![]);
            std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()))
        };
        assert!(testish(&a2, &j, "impl/whatever"));
        assert!(!testish(&a2, &j, "tests/test_x.py"));
        // An invalid override regex must not panic; it falls back to the heuristic.
        let a3 = {
            let db = crate::db::Db::mem();
            db.run("insert into rp(full,tests) values('o/r','[unclosed(')", pa![]);
            std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()))
        };
        assert!(testish(&a3, &j, "tests/test_x.py"));
        assert!(!testish(&a3, &j, "src/add.sh"));
    }

    // Adversarial: the shell guard (STAGED_GIT) is defeatable (a script the regex
    // never sees), so integrity cannot rest on it. These tests prove the controller
    // DETECTS a forged history by re-reading git state, independent of the guard.
    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_detects_forged_commit_even_if_the_shell_guard_is_bypassed() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap().status.success());
        };
        // Author (via some bypass) commits the test itself and leaves a dirty fix.
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=x", "-c", "user.email=x@y", "commit", "-qm", "test: forged"]);
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("history changed"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_green_detects_amended_red_commit() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none());
        let red_files: Vec<String> = String::from_utf8_lossy(&std::process::Command::new("git").args(["diff", "--name-only", &base, "HEAD"]).current_dir(&repo).output().unwrap().stdout).lines().map(|l| l.to_string()).collect();
        let stale = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // Author amends the committed test commit behind the controller's back.
        std::fs::write(repo.join("t/sum_test.sh"), "echo 'ok 1 - sum'\n").unwrap();
        assert!(std::process::Command::new("git").args(["-c", "user.name=x", "-c", "user.email=x@y", "commit", "-aqm", "test: amended", "--amend"]).current_dir(&repo).output().unwrap().status.success());
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        // Controller still holds the stale test_commit sha; HEAD no longer matches.
        let fb = green_commit(&a, &mut job, &red_files, &stale, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("history changed"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_sees_individual_files_in_a_new_untracked_directory() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        // A brand-new directory whose name looks test-ish, hiding production code.
        std::fs::create_dir_all(repo.join("testkit")).unwrap();
        std::fs::write(repo.join("testkit/runner.py"), "x=1\n").unwrap(); // not test-ish by name
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        let seen = changed_paths(&a, &job).await.unwrap();
        assert!(seen.iter().any(|p| p == "testkit/runner.py"), "new-dir files must be expanded: {seen:?}");
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("test files only"), "hidden non-test file must be caught: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // ===== Regression tests for the six audit blockers =====

    #[cfg(unix)]
    async fn red_ok(a: &Ax, job: &mut Job, repo: &std::path::Path) -> (String, Vec<String>) {
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(a, job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none(), "RED fixture must be accepted");
        let base = job.s("base");
        let test_commit = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(repo).output().unwrap().stdout).trim().to_string();
        let red_files: Vec<String> = String::from_utf8_lossy(&std::process::Command::new("git").args(["diff", "--name-only", &base, "HEAD"]).current_dir(repo).output().unwrap().stdout).lines().map(|l| l.to_string()).collect();
        (test_commit, red_files)
    }

    // B1: a GREEN that disables the runner (so the bug persists) is rejected by the
    // positive-execution proof, even though the test command exits 0.
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_green_cannot_pass_by_neutering_the_runner() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // (a) The "fix" leaves add.sh buggy and rewrites the runner named in the
        // frozen command to a no-op: caught because the runner is a protected path.
        std::fs::write(repo.join("t/run.sh"), "echo tests passed\nexit 0\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "neutering the runner must be rejected: {fb:?}");
        // (b) Positive-proof backstop: a command that exits 0 without ever running
        // the named test (no protected file touched) is still rejected.
        std::fs::write(repo.join("t/run.sh"), "e=0\nfor f in t/*_test.sh; do bash \"$f\" || e=1; done\nexit $e\n").unwrap(); // restore runner
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap(); // real production change
        let fb2 = green_commit(&a, &mut job, &red_files, &test_commit, "true", &["sum".to_string()], "x").await.unwrap();
        assert!(fb2.as_deref().unwrap_or("").contains("ran"), "a command that never runs the named test must fail positive proof: {fb2:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B2: a test command that mutates production after classification must not get
    // that production change committed into the RED (test-only) commit.
    #[cfg(unix)]
    #[tokio::test]
    async fn b2_test_command_mutating_production_is_rejected_not_committed() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        // The test command writes to production as a side effect.
        let tc = "printf 'add() { echo $(( $1 + $2 )); }\\n' > add.sh; bash t/run.sh";
        let fb = red_commit(&a, &mut job, tc, &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("non-test files"), "production mutation by test_cmd must be caught: {fb:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{}..HEAD", job.s("base"))]).current_dir(&repo).output().unwrap().stdout).trim(), "0", "no commit on rejection");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B3: renaming production into a test directory must be detected via the rename
    // origin, not accepted as a pure test change.
    #[cfg(unix)]
    #[tokio::test]
    async fn b3_rename_of_production_into_tests_is_detected() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        // Also add a real failing test so only the rename is the offending change.
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(std::process::Command::new("git").args(["mv", "add.sh", "t/add_test.sh"]).current_dir(&repo).output().unwrap().status.success());
        let seen = changed_paths(&a, &job).await.unwrap();
        assert!(seen.iter().any(|p| p == "add.sh"), "rename origin must be surfaced: {seen:?}");
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("test files only"), "production rename must be rejected: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B4: a test_cmd that runs git history commands is refused (the run_wt path no
    // longer bypasses the history guard).
    #[cfg(unix)]
    #[tokio::test]
    async fn b4_test_cmd_running_git_is_refused() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        let fb = red_commit(&a, &mut job, "git reset --hard HEAD~1; bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("only run the tests"), "git in test_cmd must be refused: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B5 is covered by staged_mode_is_opt_in_and_scoped (FixFlow + fail-closed).

    // B6: GREEN cannot change the proven test command or test names; the controller
    // freezes the RED values and the verifier uses only those.
    #[cfg(unix)]
    #[tokio::test]
    async fn b6_green_cannot_drift_test_cmd_or_names() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // The frozen RED command is "bash t/run.sh" / ["sum"]. Even if GREEN tried to
        // substitute a trivially-passing command, green_commit is called with the
        // FROZEN values (as staged_author does), so the drift has no effect: the real
        // runner must still pass. Here we prove the frozen values are what run.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.is_none(), "frozen command must drive GREEN: {fb:?}");
        // The persisted checkpoint records the immutable proof identity.
        job.d["staged_red"] = json!({"test_cmd":"bash t/run.sh","tests":["sum"],"test_commit":test_commit});
        assert_eq!(job.d["staged_red"]["test_cmd"], "bash t/run.sh");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // A4: REPRO and other agents keep an unrestricted shell; only the FIXER phase
    // asks run with the staged guard.
    #[cfg(unix)]
    #[tokio::test]
    async fn a4_repro_shell_is_not_staged() {
        let (a, id, dd, _repo) = staged_fixture();
        let job = a.db.jg(id).unwrap();
        assert!(!rn(&a, &job).unwrap().staged, "rn() must never pre-set the staged guard");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[test]
    fn parse_status_z_handles_modify_rename_untracked_and_spaces() {
        assert_eq!(parse_status_z(" M add.sh\0"), vec!["add.sh"]);
        assert_eq!(parse_status_z("R  test/x.sh\0src/x.sh\0"), vec!["test/x.sh", "src/x.sh"]);
        assert_eq!(parse_status_z("?? new file.sh\0"), vec!["new file.sh"]);
        assert!(parse_status_z("").is_empty());
    }

    // ===== v2 audit: P0-1, P0-2, P1-3, P2-5 =====

    // P0-1: the classic `npm test` runner-swap. GREEN leaves the bug and rewrites
    // package.json so `npm test` becomes a no-op. The controller rejects the change
    // to the runner config BEFORE running anything, so no fix commit is created and
    // the final gate never sees a false green. No Node needed: package.json is a
    // protected runner-config file.
    #[cfg(unix)]
    #[tokio::test]
    async fn p0_1_green_cannot_pass_by_editing_package_json_runner() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("package.json"), "{\"scripts\":{\"test\":\"node --test\"}}\n").unwrap();
        // commit package.json into the base so it is a tracked production/runner file
        for args in [vec!["add", "."], vec!["-c", "user.name=x", "-c", "user.email=x@y", "commit", "-qm", "add runner"]] {
            assert!(std::process::Command::new("git").args(&args).current_dir(&repo).output().unwrap().status.success());
        }
        let base = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        job.d["base"] = json!(base);
        a.db.jd(&job);
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // GREEN: bug stays; neuter the runner via package.json.
        std::fs::write(repo.join("package.json"), "{\"scripts\":{\"test\":\"echo sum passed\"}}\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "npm test", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "package.json runner edit must be rejected: {fb:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{test_commit}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "0", "no fix commit may form");
        assert!(runner_cfg("package.json") && runner_cfg("a/b/pyproject.toml") && runner_cfg("jest.config.js") && !runner_cfg("src/add.sh"));
        std::fs::remove_dir_all(dd).unwrap();
    }

    // P0-2: in staged mode the FIXER is the configured model (Qwen in the pilot),
    // never a free-first routing model, and the REVIEWER stays the configured one
    // (Claude), both independent of free_first. Checks the selection seam directly.
    #[test]
    fn p0_2_staged_uses_configured_fixer_and_reviewer_not_free_first() {
        let db = crate::db::Db::mem();
        // Pilot config: explicit Qwen fixer and Claude reviewer, free_first OFF.
        db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"],"staged_fallback":"codex:gpt-6-luna","md":{"fixer":"ollama:qwen3:8b","reviewer":"cc:opus","big":"ollama:qwen3:8b"}}));
        let a = std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()));
        let job = crate::db::Job::of(json!({"repo":"o/r","kind":"fix","d":{}}));
        assert!(staged(&a, &job));
        assert!(!crate::routing::enabled(&a), "pilot runs with free-first off");
        assert_eq!(a.md("fixer"), "ollama:qwen3:8b", "FIXER must be the configured Qwen");
        assert_eq!(a.md("reviewer"), "cc:opus", "REVIEWER must be the configured Claude");
        let visible = staged_ui_config(&a);
        assert_eq!(visible["active"], true, "the Control Room must see the active staged route");
        assert_eq!(visible["primary"], "ollama:qwen3:8b");
        assert_eq!(visible["fallback"], "codex:gpt-6-luna");
        assert_eq!(visible["fallback_effort"], "max");
        assert_eq!(visible["reviewer"], "cc:opus");
        assert_eq!(visible["repos"][0], "o/r");
        // Even if free_first were on, staged still selects a.md('fixer') in fix();
        // a.md itself returns the free model under free_first, which is exactly why
        // the pilot keeps free_first off. Document the invariant with routing off.
        assert!(!crate::routing::is_free("ollama:qwen3:8b"), "Qwen is not a free-tier routing model");
    }

    // P1-3: after a restart with a valid RED-only checkpoint, staged_resume keeps
    // the RED commit (Green) instead of rebuilding it; an invalid checkpoint is
    // discarded (Fresh).
    #[cfg(unix)]
    #[tokio::test]
    async fn p1_3_resume_reuses_valid_red_checkpoint_and_discards_invalid() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        job.d["staged_red"] = json!({"test_commit":test_commit,"test_cmd":"bash t/run.sh","tests":["sum"],"red_files":red_files,"title":"t"});
        a.db.jd(&job);
        let br = job.s("br");
        match staged_resume(&a, &mut job, &base, &br).await.unwrap() {
            Resume::Green(s) => assert_eq!(s.test_commit, test_commit, "resume must keep the RED commit"),
            _ => panic!("valid RED-only checkpoint must resume as Green"),
        }
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim(), test_commit, "branch stays on the RED commit, not reset to base");
        // A stale checkpoint whose commit never materialised AND whose branch is at
        // the base is safely discarded -> Fresh (nothing accessible to lose).
        std::process::Command::new("git").args(["reset", "-q", "--hard", &base]).current_dir(&repo).output().unwrap();
        job.d["staged_red"] = json!({"test_commit":"0000000000000000000000000000000000000000","test_cmd":"bash t/run.sh","tests":["sum"],"stage":"pending"});
        a.db.jd(&job);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await.unwrap(), Resume::Fresh));
        assert!(job.d["staged_red"].is_null(), "stale checkpoint at base is discarded");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // P2-5: commit_scope requires the staged set to EQUAL the authorised set, so an
    // authorised path that silently fails to stage is caught (not just extras).
    #[cfg(unix)]
    #[tokio::test]
    async fn p2_5_commit_scope_rejects_missing_authorised_path() {
        let (a, id, dd, repo) = staged_fixture();
        let job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("real.sh"), "x\n").unwrap();
        // Authorise a real changed file plus a phantom that does not exist -> add of
        // the phantom stages nothing for it, so the sets differ and it is rejected.
        let fb = commit_scope(&a, &job, &["real.sh".to_string()], "test: x").await;
        assert!(fb.is_ok() && fb.unwrap().is_none(), "exact set commits cleanly");
        std::fs::write(repo.join("real2.sh"), "y\n").unwrap();
        let fb = commit_scope(&a, &job, &["real2.sh".to_string(), "ghost.sh".to_string()], "test: y").await.unwrap();
        let m = fb.as_deref().unwrap_or("");
        assert!(m.contains("could not stage") || m.contains("unexpected staging state"), "missing authorised path must be caught: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // P2-6 (partial, no live model): drive the controller phases as a simulated
    // author across a failed GREEN then a successful GREEN, proving the RED commit
    // SHA is preserved across the retry and exactly two commits result.
    #[cfg(unix)]
    #[tokio::test]
    async fn p2_6_simulated_author_red_then_failed_then_passing_green() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // GREEN attempt 1 (simulated author): writes an unrelated non-fix -> rejected.
        std::fs::write(repo.join("noop.sh"), "true\n").unwrap();
        let fb1 = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb1.is_some(), "a non-fix GREEN must be rejected");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim(), test_commit, "RED commit preserved after failed GREEN");
        // Reset working tree to the RED commit (as staged_author does between tries).
        for args in [vec!["checkout", "-q", "-f", &test_commit], vec!["clean", "-fdq"]] {
            std::process::Command::new("git").args(&args).current_dir(&repo).output().unwrap();
        }
        // GREEN attempt 2 (simulated author): the real fix -> accepted.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb2 = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "fix add").await.unwrap();
        assert!(fb2.is_none(), "the real fix must be accepted: {fb2:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "2");
        assert_red_then_green(&repo);
        std::fs::remove_dir_all(dd).unwrap();
    }

    // Deterministic RED->GREEN check via git directly (no ok()-retry contention):
    // the test fails on the test commit (HEAD~1) and passes on the fix commit (HEAD).
    #[cfg(unix)]
    fn assert_red_then_green(repo: &std::path::Path) {
        let run = |rev: &str| {
            std::process::Command::new("git").args(["checkout", "-q", "-f", rev]).current_dir(repo).output().unwrap();
            std::process::Command::new("bash").arg("t/run.sh").current_dir(repo).output().unwrap().status.code()
        };
        let head = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(repo).output().unwrap().stdout).trim().to_string();
        assert_eq!(run("HEAD~1"), Some(1), "RED: test must fail on the test commit");
        assert_eq!(run(&head), Some(0), "GREEN: test must pass on the fix commit");
    }

    // ===== v3 audit: V3-P0-1, V3-P1-2, V3-P1-3, V3-P2-4 =====

    // V3-P0-1: the INDIRECT runner attack. `npm test` -> `sh scripts/check.sh`.
    // GREEN leaves production wrong and rewrites scripts/check.sh to print the test
    // name. The mandatory production allowlist (only src/calc.cjs) rejects the
    // change to scripts/check.sh before anything runs; no fix commit forms.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p0_1_green_cannot_edit_indirect_runner_script() {
        let dd = std::env::temp_dir().join(format!("apollo-v3runner-{}", hex::encode(rand::random::<[u8; 8]>())));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(repo.join("test")).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        std::fs::write(repo.join("src/calc.cjs"), "exports.subtract = (a,b) => a+b;\n").unwrap();
        std::fs::write(repo.join("package.json"), "{\"scripts\":{\"test\":\"sh scripts/check.sh\"}}\n").unwrap();
        std::fs::write(repo.join("scripts/check.sh"), "node --test test/subtraction.test.cjs\n").unwrap();
        std::fs::write(repo.join("t_placeholder"), "x").unwrap();
        git(&["init", "-q", "-b", "work"]);
        git(&["config", "user.name", "x"]);
        git(&["config", "user.email", "x@y"]);
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        let base = git(&["rev-parse", "HEAD"]);
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"]}));
        db.run("insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)", pa![]);
        // Production allowlist: ONLY src/calc.cjs.
        db.run("insert into rp(full,green,tests) values('o/r','^src/calc\\.cjs$','(^|/)test/')", pa![]);
        let id = db.jnew("o/r", 0, "fix", "subtraction", "", "fix", "", json!({"h":"local","base":base,"br":"work","repro":{"how":"x"}}), None, 0.0);
        let a = std::sync::Arc::new(crate::app::App::new(db, dd.clone(), 1, "apollo-office".into()));
        let mut job = a.db.jg(id).unwrap();
        // RED: a real failing test under test/ (node assertion).
        std::fs::write(repo.join("test/subtraction.test.cjs"), "const {test}=require('node:test');const assert=require('node:assert/strict');const {subtract}=require('../src/calc.cjs');\ntest('subtraction',()=>assert.equal(subtract(7,3),4));\n").unwrap();
        let red = red_commit(&a, &mut job, "npm test", &["subtraction".to_string()], "cover subtraction").await.unwrap();
        assert!(red.is_none(), "RED must be accepted: {red:?}");
        let test_commit = git(&["rev-parse", "HEAD"]);
        let red_files = vec!["test/subtraction.test.cjs".to_string()];
        // GREEN: do NOT fix src/calc.cjs; neuter the indirect runner script.
        std::fs::write(repo.join("scripts/check.sh"), "echo subtraction\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "npm test", &["subtraction".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "indirect runner edit must be rejected: {fb:?}");
        assert_eq!(git(&["rev-list", "--count", &format!("{test_commit}..HEAD")]), "0", "no fix commit may form");
        assert_eq!(std::fs::read_to_string(repo.join("src/calc.cjs")).unwrap().trim(), "exports.subtract = (a,b) => a+b;", "production stays buggy, not falsely green");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P0-1 (fail-closed): a staged repo with NO green allowlist cannot run GREEN.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p0_1_green_requires_mandatory_allowlist() {
        let (a, id, dd, repo) = staged_fixture();
        a.db.run("update rp set green='' where full='o/r'", pa![]); // remove allowlist
        let mut job = a.db.jg(id).unwrap();
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("requires a production-file allowlist"), "missing allowlist must fail closed: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P1-2: a crash after the GREEN commit but before the checkpoint is cleared
    // must NOT discard the commit. staged_resume returns Finished and the two
    // commits stay on the branch.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p1_2_resume_after_green_commit_keeps_both_commits() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        assert!(green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "fix").await.unwrap().is_none());
        let green = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // Simulate the crash window: both checkpoints persisted, not yet cleared.
        job.d["staged_red"] = json!({"test_commit":test_commit,"test_cmd":"bash t/run.sh","tests":["sum"],"red_files":red_files,"title":"fix"});
        job.d["staged_green"] = json!({"commit":green,"js":{"title":"fix","test_cmd":"bash t/run.sh","tests":["sum"],"cause":"c","fix":"f"}});
        a.db.jd(&job);
        let br = job.s("br");
        match staged_resume(&a, &mut job, &base, &br).await.unwrap() {
            Resume::Finished(js) => assert_eq!(js["title"], "fix"),
            _ => panic!("must recover the finished RED+GREEN pair"),
        }
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "2", "both commits preserved, no destructive reset");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim(), green, "HEAD still the GREEN commit");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P1-3: fix() refuses to run staged under free-first routing (fail closed),
    // rather than silently substituting the configured FIXER/REVIEWER.
    #[test]
    fn v3_p1_3_free_first_on_is_a_blocking_conflict() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true,"staged_tdd":true,"staged_repos":["o/r"]}));
        let a = std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()));
        let job = crate::db::Job::of(json!({"repo":"o/r","kind":"fix","d":{}}));
        assert!(staged_conflict(&a, &job), "intended + free_first on is a conflict fix() rejects");
        assert!(!staged(&a, &job), "and the mode is not active");
    }

    // V3-P2-4: drive the whole staged_author state machine with a SIMULATED agent
    // (no real model): a RED turn, a GREEN give_up (retry), then a passing GREEN.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p2_4_staged_author_statemachine_with_simulated_agent() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        {
            let mut q = a.sim.lock().unwrap();
            q.push_back(json!({"_files":{"t/sum_test.sh": RED_FAIL}, "test_cmd":"bash t/run.sh", "tests":["sum"], "title":"cover sum"}));
            q.push_back(json!({"give_up":"not sure yet"})); // GREEN attempt 1 -> retry
            q.push_back(json!({"_files":{"add.sh":"add() { echo $(( $1 + $2 )); }\n"}, "title":"add not subtract"})); // GREEN attempt 2 -> pass
        }
        let mut fb = String::new();
        let out = staged_author(&a, &mut job, &mut fb, "ollama:qwen3:8b").await.unwrap();
        assert!(out.is_some(), "simulated RED + retried GREEN must complete: fb={fb}");
        let js = out.unwrap();
        assert_eq!(js["test_cmd"], "bash t/run.sh");
        assert_eq!(js["tests"][0], "sum");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "2");
        assert_red_then_green(&repo);
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P2-4: an invalid (no-JSON) RED answer from the simulated agent is handled.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p2_4_staged_author_handles_invalid_red_json() {
        let (a, id, dd, _repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        a.sim.lock().unwrap().push_back(Value::Null); // RED returns no JSON
        let mut fb = String::new();
        let out = staged_author(&a, &mut job, &mut fb, "ollama:qwen3:8b").await.unwrap();
        assert!(out.is_none(), "no-JSON RED must not complete");
        assert!(fb.contains("no JSON"), "fb={fb}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // ===== v4 audit: crash-window state recovery =====

    #[cfg(unix)]
    fn head_sha(repo: &std::path::Path) -> String {
        String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(repo).output().unwrap().stdout).trim().to_string()
    }
    #[cfg(unix)]
    fn count_above(repo: &std::path::Path, base: &str) -> String {
        String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(repo).output().unwrap().stdout).trim().to_string()
    }
    #[cfg(unix)]
    fn commit_present(repo: &std::path::Path, sha: &str) -> bool {
        std::process::Command::new("git").args(["cat-file", "-e", &format!("{sha}^{{commit}}")]).current_dir(repo).output().unwrap().status.success()
    }

    // V4 Window A: crash AFTER the RED commit but BEFORE the checkpoint is finalised
    // leaves a PENDING checkpoint (tc/ts, no test_commit). staged_resume must bind it
    // to the real RED commit and continue GREEN — the commit stays accessible, no reset.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_window_a_crash_after_red_commit_recovers_without_reset() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (red, _red_files) = red_ok(&a, &mut job, &repo).await;
        // Simulate the crash window: only the PENDING checkpoint exists (no test_commit).
        job.d["staged_red"] = json!({"test_cmd":"bash t/run.sh","tests":["sum"],"title":"t","stage":"pending"});
        job.d.as_object_mut().unwrap().remove("staged_green");
        a.db.jd(&job);
        let br = job.s("br");
        match staged_resume(&a, &mut job, &base, &br).await.unwrap() {
            Resume::Green(s) => assert_eq!(s.test_commit, red, "must bind to the real RED commit"),
            r => panic!("window A must recover as Green, got {}", matches!(r, Resume::Fresh)),
        }
        assert_eq!(head_sha(&repo), red, "RED commit stays on the branch (no reset)");
        assert_eq!(count_above(&repo, &base), "1");
        assert!(commit_present(&repo, &red));
        assert_eq!(job.d["staged_red"]["test_commit"], red, "checkpoint finalised on recovery");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V4 Window B: a crash that leaves RED+GREEN committed but NO checkpoint and NO
    // discard authorisation must BLOCK (not reset): both commits stay accessible.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_window_b_crash_without_checkpoint_blocks_without_reset() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        assert!(green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "fix").await.unwrap().is_none());
        let green = head_sha(&repo);
        // Crash state: both checkpoints gone, no discard marker.
        job.d.as_object_mut().unwrap().remove("staged_red");
        job.d.as_object_mut().unwrap().remove("staged_green");
        job.d.as_object_mut().unwrap().remove("staged_discard");
        a.db.jd(&job);
        let br = job.s("br");
        let r = staged_resume(&a, &mut job, &base, &br).await;
        assert!(matches!(r, Err(X::Drop(_))), "must block for manual inspection");
        assert_eq!(head_sha(&repo), green, "HEAD unchanged; no destructive reset");
        assert_eq!(count_above(&repo, &base), "2", "both commits remain accessible");
        assert!(commit_present(&repo, &test_commit) && commit_present(&repo, &green));
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V4 point 5: an INTENTIONAL rejection persists a discard authorisation for the
    // exact HEAD, so the next round may reset (Fresh); a crash (no such marker) blocks.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_intentional_rejection_authorises_reset_but_crash_blocks() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (_tc, _rf) = red_ok(&a, &mut job, &repo).await;
        let head = head_sha(&repo);
        let br = job.s("br");
        // No authorisation yet (simulated crash): commit present, no checkpoint -> block.
        job.d.as_object_mut().unwrap().remove("staged_red");
        a.db.jd(&job);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await, Err(X::Drop(_))), "crash state must block");
        assert_eq!(head_sha(&repo), head, "blocked state left the branch intact");
        // Intentional rejection authorises discarding exactly this HEAD.
        reject_candidate(&a, &mut job, &head, &base);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await.unwrap(), Resume::Fresh), "authorised discard resumes Fresh");
        assert!(job.d["staged_discard"].is_null(), "authorisation consumed");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V4 point 7: absent checkpoint with the branch at the base is Fresh; an
    // incoherent checkpoint (corrupt) with commits present blocks without reset.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_absent_at_base_is_fresh_and_corrupt_with_commits_blocks() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let br = job.s("br");
        // Absent checkpoint, HEAD at base -> Fresh.
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await.unwrap(), Resume::Fresh));
        // Corrupt checkpoint (no tc) with a real commit present -> block, no reset.
        let (red, _rf) = red_ok(&a, &mut job, &repo).await;
        job.d["staged_red"] = json!({"test_commit":red,"stage":"committed"}); // missing tc/tests
        a.db.jd(&job);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await, Err(X::Drop(_))), "corrupt checkpoint with commits must block");
        assert_eq!(head_sha(&repo), red, "no reset on corrupt-with-commits");
        std::fs::remove_dir_all(dd).unwrap();
    }

}
