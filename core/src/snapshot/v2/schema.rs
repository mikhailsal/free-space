pub(super) const APP_ID: i64 = 0x46535632;
// Experimental format revisions are reseeded, never silently migrated in place.
pub(super) const REVISION: i64 = 4;
pub(super) const COLUMNS: &str = "id,parent,path,sort_key,kind,flags,own_allocated,own_apparent,own_errors,mtime,scanned_at,allocated,apparent,items,errors,min_scanned_at,newest_mtime,scope,nlink,dev,ino";
pub(super) const CREATE: &str = "
CREATE TABLE control(singleton INTEGER PRIMARY KEY CHECK(singleton=1), current INTEGER NOT NULL, prev INTEGER, full_scan INTEGER NOT NULL, one_file_system INTEGER NOT NULL);
CREATE TABLE identities(id INTEGER PRIMARY KEY AUTOINCREMENT, scope BLOB NOT NULL, path BLOB NOT NULL, UNIQUE(scope,path));
CREATE TABLE live(
 id INTEGER PRIMARY KEY, parent INTEGER, path BLOB NOT NULL, sort_key BLOB NOT NULL,
 kind INTEGER NOT NULL, flags INTEGER NOT NULL,
 own_allocated INTEGER NOT NULL, own_apparent INTEGER NOT NULL, own_errors INTEGER NOT NULL,
 mtime INTEGER NOT NULL, scanned_at INTEGER NOT NULL,
 allocated INTEGER NOT NULL, apparent INTEGER NOT NULL, items INTEGER NOT NULL, errors INTEGER NOT NULL,
 min_scanned_at INTEGER NOT NULL, newest_mtime INTEGER,
 scope BLOB NOT NULL, nlink INTEGER NOT NULL, dev INTEGER, ino INTEGER,
 UNIQUE(scope,path), UNIQUE(scope,sort_key), CHECK((dev IS NULL)=(ino IS NULL))
);
CREATE INDEX live_child_min ON live(parent,min_scanned_at);
CREATE INDEX live_child_newest ON live(parent,newest_mtime DESC);
CREATE INDEX live_inode ON live(dev,ino) WHERE dev IS NOT NULL;
CREATE TABLE versions(
 id INTEGER NOT NULL, parent INTEGER, path BLOB NOT NULL, sort_key BLOB NOT NULL,
 kind INTEGER NOT NULL, flags INTEGER NOT NULL,
 own_allocated INTEGER NOT NULL, own_apparent INTEGER NOT NULL, own_errors INTEGER NOT NULL,
 mtime INTEGER NOT NULL, scanned_at INTEGER NOT NULL,
 allocated INTEGER NOT NULL, apparent INTEGER NOT NULL, items INTEGER NOT NULL, errors INTEGER NOT NULL,
 min_scanned_at INTEGER NOT NULL, newest_mtime INTEGER,
 scope BLOB NOT NULL, nlink INTEGER NOT NULL, dev INTEGER, ino INTEGER,
 born INTEGER NOT NULL, dead INTEGER,
 PRIMARY KEY(id,born)
);
CREATE INDEX versions_gc ON versions(dead,id,born) WHERE dead IS NOT NULL;
CREATE UNIQUE INDEX versions_open ON versions(id) WHERE dead IS NULL;
CREATE INDEX versions_path ON versions(scope,path,born);
CREATE INDEX versions_subtree ON versions(scope,sort_key,born);
CREATE INDEX versions_children ON versions(parent,sort_key,born);
CREATE INDEX versions_inode ON versions(dev,ino,born) WHERE dev IS NOT NULL;
CREATE INDEX versions_denied ON versions(scope,sort_key,born) WHERE (flags & 1)!=0;
CREATE TABLE live_roots(entry_id INTEGER PRIMARY KEY,path BLOB NOT NULL UNIQUE,dev INTEGER NOT NULL,fs_type TEXT NOT NULL,total INTEGER NOT NULL,free INTEGER NOT NULL,avail INTEGER NOT NULL,used INTEGER NOT NULL,inodes_total INTEGER NOT NULL,inodes_free INTEGER NOT NULL,ordinal INTEGER NOT NULL);
CREATE TABLE root_versions(entry_id INTEGER NOT NULL,path BLOB NOT NULL,dev INTEGER NOT NULL,fs_type TEXT NOT NULL,total INTEGER NOT NULL,free INTEGER NOT NULL,avail INTEGER NOT NULL,used INTEGER NOT NULL,inodes_total INTEGER NOT NULL,inodes_free INTEGER NOT NULL,ordinal INTEGER NOT NULL,born INTEGER NOT NULL,dead INTEGER,PRIMARY KEY(entry_id,born));
CREATE INDEX roots_gc ON root_versions(dead,entry_id,born) WHERE dead IS NOT NULL;
CREATE UNIQUE INDEX roots_open ON root_versions(entry_id) WHERE dead IS NULL;
CREATE INDEX roots_path ON root_versions(path,born);
CREATE TABLE live_mounts(entry_id INTEGER PRIMARY KEY,path BLOB NOT NULL,fs_type TEXT NOT NULL,used INTEGER,total INTEGER);
CREATE TABLE mount_versions(entry_id INTEGER NOT NULL,path BLOB NOT NULL,fs_type TEXT NOT NULL,used INTEGER,total INTEGER,born INTEGER NOT NULL,dead INTEGER,PRIMARY KEY(entry_id,born));
CREATE INDEX mounts_gc ON mount_versions(dead,entry_id,born) WHERE dead IS NOT NULL;
CREATE UNIQUE INDEX mounts_open ON mount_versions(entry_id) WHERE dead IS NULL;
CREATE TABLE generation_meta(generation INTEGER PRIMARY KEY,prev_generation INTEGER,scanned_at INTEGER NOT NULL,host TEXT NOT NULL,os TEXT NOT NULL,snapshot_id TEXT NOT NULL,entries INTEGER NOT NULL,one_file_system INTEGER NOT NULL,freespace_version TEXT NOT NULL,full_scan INTEGER NOT NULL);
CREATE TABLE refreshes(generation INTEGER PRIMARY KEY,path BLOB NOT NULL,scanned_at INTEGER NOT NULL);
CREATE TABLE retention(singleton INTEGER PRIMARY KEY CHECK(singleton=1),max_records INTEGER NOT NULL,max_wal_bytes INTEGER NOT NULL,next_kind INTEGER NOT NULL);
INSERT INTO retention VALUES(1,100000000,536870912,0);
CREATE TABLE storage_stats(singleton INTEGER PRIMARY KEY CHECK(singleton=1),retained_rows INTEGER NOT NULL);
INSERT INTO storage_stats VALUES(1,0);
CREATE TABLE gc_cursor(kind INTEGER PRIMARY KEY,k1 INTEGER NOT NULL,k2 INTEGER NOT NULL,k3 INTEGER NOT NULL);
INSERT INTO gc_cursor VALUES(0,0,0,0),(1,0,0,0),(2,0,0,0),(3,0,0,0),(4,0,0,0),(5,0,0,0);
";

pub(super) fn accounting(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    for table in [
        "identities",
        "versions",
        "root_versions",
        "mount_versions",
        "generation_meta",
        "refreshes",
    ] {
        conn.execute_batch(&format!("CREATE TRIGGER {table}_account_insert AFTER INSERT ON {table} BEGIN UPDATE storage_stats SET retained_rows=retained_rows+1 WHERE singleton=1; END; CREATE TRIGGER {table}_account_delete AFTER DELETE ON {table} BEGIN UPDATE storage_stats SET retained_rows=retained_rows-1 WHERE singleton=1; END;"))?;
    }
    Ok(())
}
