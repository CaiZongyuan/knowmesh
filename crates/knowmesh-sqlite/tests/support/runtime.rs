use rusqlite::{Connection, params};

pub fn runtime_fixture(path: &std::path::Path, revision_id: &str) {
    let db = Connection::open(path).unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON; BEGIN; PRAGMA defer_foreign_keys=ON;")
        .unwrap();
    for (id, parent) in [("child", Some("parent")), ("parent", None)] {
        db.execute("INSERT INTO operation_runs(id,parent_run_id,operation,surface,actor,status,input_json,input_digest,created_at,updated_at) VALUES(?1,?2,'compile','cli','fixture','queued','{}','input','2026-09-05T00:00:00Z','2026-09-05T00:00:00Z')", params![id, parent]).unwrap();
    }
    db.execute("INSERT INTO proposals(id,kind,state,base_generation,source_revision_id,schema_hash,summary_json,created_by,created_at,updated_at) VALUES('proposal','compile','draft',1,?1,'schema','{}','fixture','2026-09-05T00:00:00Z','2026-09-05T00:00:00Z')", [revision_id]).unwrap();
    db.execute("INSERT INTO proposal_items(id,proposal_id,ordinal,op,payload_json,risk,decision) VALUES('item','proposal',0,'node.upsert','{}','low','pending')", []).unwrap();
    db.execute("INSERT INTO idempotency_keys(key,operation,input_hash,run_id,state,response_json,status_code,created_at) VALUES('fixture-key','compile','input','child','completed','{\"output\":\"proposal\"}',200,'2026-09-05T00:00:00Z')", []).unwrap();
    db.execute("INSERT INTO audit_events(event_id,run_id,event_type,actor,created_at) VALUES('event','child','run.finished','fixture','2026-09-05T00:00:00Z')", []).unwrap();
    db.execute("INSERT INTO audit_events(seq,event_id,event_type,actor,created_at) VALUES(90,'deleted-event','fixture','fixture','2026-09-05T00:00:00Z')", []).unwrap();
    db.execute("DELETE FROM audit_events WHERE seq=90", [])
        .unwrap();
    db.execute_batch("COMMIT;").unwrap();
}
