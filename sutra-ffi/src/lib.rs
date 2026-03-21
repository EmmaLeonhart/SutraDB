//! C-compatible FFI layer for SutraDB.
//!
//! This crate produces a shared library (`.dll`/`.so`/`.dylib`) that can be
//! loaded by any language with FFI support — primarily Dart (`dart:ffi`) for
//! Sutra Studio, but also usable from Python (`ctypes`), C, C++, etc.
//!
//! # Architecture
//!
//! The FFI library is designed so that **one process can host everything**:
//! - The database engine (sutra-core, sutra-hnsw, sutra-sparql)
//! - The MCP server (optional, started via `sutra_mcp_start`)
//! - The GUI (Sutra Studio loads this library via dart:ffi)
//!
//! All three share the same database handle. The MCP server runs on a
//! background thread when started. The GUI is entirely in Flutter — this
//! library just provides the database operations.
//!
//! # Memory Management
//!
//! - Opaque handles (`SutraDb`, `SutraResult`) are heap-allocated and must
//!   be freed by the corresponding `_free` function.
//! - Strings returned by `sutra_resolve`, `sutra_health_report`, etc. are
//!   owned C strings that must be freed with `sutra_string_free`.
//! - All functions are thread-safe — the opaque handle wraps `Arc<Mutex<...>>`.
//!
//! # Error Handling
//!
//! Functions return null pointers or negative integers on error. Call
//! `sutra_last_error()` to get the error message.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::{Arc, Mutex};

// ─── Thread-local last error ─────────────────────────────────────────────────

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

/// Get the last error message. Returns null if no error.
/// The returned string is valid until the next FFI call on this thread.
#[no_mangle]
pub extern "C" fn sutra_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        e.borrow()
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}

// ─── Opaque types ────────────────────────────────────────────────────────────

/// Opaque database handle. Thread-safe, shareable across FFI boundary.
struct DbInner {
    ps: sutra_core::PersistentStore,
    store: sutra_core::TripleStore,
    dict: sutra_core::TermDictionary,
    vectors: sutra_hnsw::VectorRegistry,
}

/// Opaque handle passed across the FFI boundary.
pub struct SutraDb {
    inner: Arc<Mutex<DbInner>>,
}

/// Opaque query result handle.
pub struct SutraResult {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

// ─── Database lifecycle ──────────────────────────────────────────────────────

/// Open (or create) a SutraDB database at the given path.
///
/// Returns an opaque handle, or null on error (call `sutra_last_error()`).
#[no_mangle]
pub extern "C" fn sutra_db_open(path: *const c_char) -> *mut SutraDb {
    let path_str = match unsafe_cstr_to_str(path) {
        Some(s) => s,
        None => {
            set_error("Invalid or null path");
            return std::ptr::null_mut();
        }
    };

    let ps = match sutra_core::PersistentStore::open(path_str) {
        Ok(ps) => ps,
        Err(e) => {
            set_error(&format!("Failed to open database: {}", e));
            return std::ptr::null_mut();
        }
    };

    let mut dict = sutra_core::TermDictionary::new();
    ps.load_terms_into(&mut dict);

    let mut store = sutra_core::TripleStore::new();
    for triple in ps.iter() {
        let _ = store.insert(triple);
    }

    // Rebuild HNSW indexes from stored vector triples
    let mut vectors = sutra_hnsw::VectorRegistry::new();
    let f32vec_suffix = "^^<http://sutra.dev/f32vec>";
    for triple in store.iter() {
        if let Some(obj_str) = dict.resolve(triple.object) {
            if obj_str.contains(f32vec_suffix) {
                if let Some(start) = obj_str.find('"') {
                    let end = obj_str[start + 1..].find('"').map(|p| p + start + 1);
                    if let Some(end) = end {
                        let vec_str = &obj_str[start + 1..end];
                        let floats: Vec<f32> = vec_str
                            .split_whitespace()
                            .filter_map(|s| s.parse::<f32>().ok())
                            .collect();
                        if !floats.is_empty() {
                            let dims = floats.len();
                            if !vectors.has_index(triple.predicate) {
                                let config = sutra_hnsw::VectorPredicateConfig {
                                    predicate_id: triple.predicate,
                                    dimensions: dims,
                                    m: 16,
                                    ef_construction: 200,
                                    metric: sutra_hnsw::DistanceMetric::Cosine,
                                };
                                let _ = vectors.declare(config);
                            }
                            let _ = vectors.insert(triple.predicate, floats, triple.object);
                        }
                    }
                }
            }
        }
    }

    let db = SutraDb {
        inner: Arc::new(Mutex::new(DbInner {
            ps,
            store,
            dict,
            vectors,
        })),
    };

    Box::into_raw(Box::new(db))
}

/// Close a database and free the handle.
///
/// After this call, the handle is invalid. Passing it to any other function
/// is undefined behavior.
#[no_mangle]
pub extern "C" fn sutra_db_close(db: *mut SutraDb) {
    if !db.is_null() {
        unsafe {
            let db = Box::from_raw(db);
            let flush_result = db.inner.lock().map(|inner| inner.ps.flush());
            drop(flush_result);
            drop(db);
        }
    }
}

// ─── Triple operations ───────────────────────────────────────────────────────

/// Get the total number of triples in the database.
#[no_mangle]
pub extern "C" fn sutra_triple_count(db: *const SutraDb) -> u64 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => return 0,
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    inner.store.len() as u64
}

/// Get the number of terms in the dictionary.
#[no_mangle]
pub extern "C" fn sutra_term_count(db: *const SutraDb) -> u64 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => return 0,
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    inner.dict.len() as u64
}

/// Insert triples in N-Triples format.
///
/// Returns the number of triples inserted, or -1 on error.
#[no_mangle]
pub extern "C" fn sutra_insert_ntriples(db: *mut SutraDb, data: *const c_char) -> i64 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => {
            set_error("Null database handle");
            return -1;
        }
    };
    let data_str = match unsafe_cstr_to_str(data) {
        Some(s) => s,
        None => {
            set_error("Invalid or null data");
            return -1;
        }
    };

    let mut inner = match db.inner.lock() {
        Ok(i) => i,
        Err(e) => {
            set_error(&format!("Lock error: {}", e));
            return -1;
        }
    };

    let mut inserted = 0i64;
    for line in data_str.lines() {
        let parsed = match sutra_core::parse_ntriples_line(line) {
            Some(t) => t,
            None => continue,
        };
        let (subj_str, pred_str, obj_str) = parsed;
        let s_id = match inner.ps.intern(&subj_str) {
            Ok(id) => {
                inner.dict.insert_with_id(&subj_str, id);
                id
            }
            Err(e) => {
                set_error(&format!("Intern error: {}", e));
                return -1;
            }
        };
        let p_id = match inner.ps.intern(&pred_str) {
            Ok(id) => {
                inner.dict.insert_with_id(&pred_str, id);
                id
            }
            Err(e) => {
                set_error(&format!("Intern error: {}", e));
                return -1;
            }
        };
        let o_id = match inner.ps.intern(&obj_str) {
            Ok(id) => {
                inner.dict.insert_with_id(&obj_str, id);
                id
            }
            Err(e) => {
                set_error(&format!("Intern error: {}", e));
                return -1;
            }
        };
        let triple = sutra_core::Triple::new(s_id, p_id, o_id);
        if inner.ps.insert(triple).is_ok() {
            let _ = inner.store.insert(triple);
            inserted += 1;
        }
    }

    if let Err(e) = inner.ps.flush() {
        set_error(&format!("Flush error: {}", e));
        return -1;
    }

    inserted
}

// ─── Term dictionary ─────────────────────────────────────────────────────────

/// Intern a term string and return its ID. Returns 0 on error.
#[no_mangle]
pub extern "C" fn sutra_intern(db: *mut SutraDb, term: *const c_char) -> u64 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => return 0,
    };
    let term_str = match unsafe_cstr_to_str(term) {
        Some(s) => s,
        None => return 0,
    };
    let mut inner = match db.inner.lock() {
        Ok(i) => i,
        Err(_) => return 0,
    };
    match inner.ps.intern(term_str) {
        Ok(id) => {
            inner.dict.insert_with_id(term_str, id);
            id
        }
        Err(_) => 0,
    }
}

/// Resolve a term ID to its string representation.
///
/// Returns a C string that must be freed with `sutra_string_free`.
/// Returns null if the ID is unknown.
#[no_mangle]
pub extern "C" fn sutra_resolve(db: *const SutraDb, id: u64) -> *mut c_char {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => return std::ptr::null_mut(),
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(_) => return std::ptr::null_mut(),
    };

    // Check inline types first
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return string_to_c(&n.to_string());
    }
    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return string_to_c(&b.to_string());
    }

    inner
        .dict
        .resolve(id)
        .map(|s| string_to_c(s))
        .unwrap_or(std::ptr::null_mut())
}

// ─── SPARQL query ────────────────────────────────────────────────────────────

/// Execute a SPARQL+ query and return a result handle.
///
/// Returns null on error (call `sutra_last_error()`).
/// The result must be freed with `sutra_result_free`.
#[no_mangle]
pub extern "C" fn sutra_query(db: *const SutraDb, query: *const c_char) -> *mut SutraResult {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => {
            set_error("Null database handle");
            return std::ptr::null_mut();
        }
    };
    let query_str = match unsafe_cstr_to_str(query) {
        Some(s) => s,
        None => {
            set_error("Invalid or null query");
            return std::ptr::null_mut();
        }
    };

    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(e) => {
            set_error(&format!("Lock error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let mut parsed = match sutra_sparql::parse(query_str) {
        Ok(q) => q,
        Err(e) => {
            set_error(&format!("SPARQL parse error: {}", e));
            return std::ptr::null_mut();
        }
    };

    sutra_sparql::optimize(&mut parsed);

    let result =
        match sutra_sparql::execute_with_vectors(&parsed, &inner.store, &inner.dict, &inner.vectors)
        {
            Ok(r) => r,
            Err(e) => {
                set_error(&format!("SPARQL execution error: {}", e));
                return std::ptr::null_mut();
            }
        };

    // Convert to string-resolved rows for easy consumption across FFI
    let columns = result.columns.clone();
    let rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|col| {
                    row.get(col)
                        .map(|&id| resolve_id(id, &inner.dict))
                        .unwrap_or_default()
                })
                .collect()
        })
        .collect();

    Box::into_raw(Box::new(SutraResult { columns, rows }))
}

/// Get the number of columns in a result.
#[no_mangle]
pub extern "C" fn sutra_result_column_count(result: *const SutraResult) -> u32 {
    if result.is_null() {
        return 0;
    }
    unsafe { (*result).columns.len() as u32 }
}

/// Get the number of rows in a result.
#[no_mangle]
pub extern "C" fn sutra_result_row_count(result: *const SutraResult) -> u64 {
    if result.is_null() {
        return 0;
    }
    unsafe { (*result).rows.len() as u64 }
}

/// Get a column name by index.
///
/// Returns a C string that must be freed with `sutra_string_free`.
#[no_mangle]
pub extern "C" fn sutra_result_column_name(
    result: *const SutraResult,
    index: u32,
) -> *mut c_char {
    if result.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { &*result };
    result
        .columns
        .get(index as usize)
        .map(|s| string_to_c(s))
        .unwrap_or(std::ptr::null_mut())
}

/// Get a cell value by row and column index.
///
/// Returns a C string that must be freed with `sutra_string_free`.
#[no_mangle]
pub extern "C" fn sutra_result_value(
    result: *const SutraResult,
    row: u64,
    col: u32,
) -> *mut c_char {
    if result.is_null() {
        return std::ptr::null_mut();
    }
    let result = unsafe { &*result };
    result
        .rows
        .get(row as usize)
        .and_then(|r| r.get(col as usize))
        .map(|s| string_to_c(s))
        .unwrap_or(std::ptr::null_mut())
}

/// Free a query result handle.
#[no_mangle]
pub extern "C" fn sutra_result_free(result: *mut SutraResult) {
    if !result.is_null() {
        unsafe {
            drop(Box::from_raw(result));
        }
    }
}

// ─── Health diagnostics ──────────────────────────────────────────────────────

/// Generate a full health report as structured text.
///
/// Returns a C string that must be freed with `sutra_string_free`.
#[no_mangle]
pub extern "C" fn sutra_health_report(db: *const SutraDb) -> *mut c_char {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => {
            set_error("Null database handle");
            return std::ptr::null_mut();
        }
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(e) => {
            set_error(&format!("Lock error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let report =
        sutra_sparql::generate_health_report(&inner.store, &inner.dict, &inner.vectors, None);
    string_to_c(&report.to_ai_text())
}

/// Get the overall health status: 0 = healthy, 1 = warning, 2 = critical.
#[no_mangle]
pub extern "C" fn sutra_health_status(db: *const SutraDb) -> i32 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => return -1,
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(_) => return -1,
    };
    let report =
        sutra_sparql::generate_health_report(&inner.store, &inner.dict, &inner.vectors, None);
    match report.overall_status {
        sutra_sparql::HealthStatus::Healthy => 0,
        sutra_sparql::HealthStatus::Warning => 1,
        sutra_sparql::HealthStatus::Critical => 2,
    }
}

// ─── Database info ───────────────────────────────────────────────────────────

/// Get database info as a JSON string.
///
/// Returns a C string that must be freed with `sutra_string_free`.
#[no_mangle]
pub extern "C" fn sutra_db_info(db: *const SutraDb) -> *mut c_char {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => {
            set_error("Null database handle");
            return std::ptr::null_mut();
        }
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(e) => {
            set_error(&format!("Lock error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let info = format!(
        "{{\"triples\":{},\"terms\":{},\"vector_predicates\":{}}}",
        inner.store.len(),
        inner.dict.len(),
        inner.vectors.predicates().len()
    );
    string_to_c(&info)
}

// ─── Index consistency ───────────────────────────────────────────────────────

/// Verify SPO/POS/OSP index consistency. Returns 1 if consistent, 0 if not.
#[no_mangle]
pub extern "C" fn sutra_verify_consistency(db: *const SutraDb) -> i32 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => return -1,
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(_) => return -1,
    };
    if inner.ps.verify_consistency() {
        1
    } else {
        0
    }
}

/// Repair index inconsistencies. Returns the number of triples repaired, or -1 on error.
#[no_mangle]
pub extern "C" fn sutra_repair(db: *mut SutraDb) -> i64 {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => {
            set_error("Null database handle");
            return -1;
        }
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(e) => {
            set_error(&format!("Lock error: {}", e));
            return -1;
        }
    };
    match inner.ps.repair() {
        Ok(count) => {
            let _ = inner.ps.flush();
            count as i64
        }
        Err(e) => {
            set_error(&format!("Repair failed: {}", e));
            -1
        }
    }
}

// ─── Export ───────────────────────────────────────────────────────────────────

/// Export all triples as N-Triples text.
///
/// Returns a C string that must be freed with `sutra_string_free`.
#[no_mangle]
pub extern "C" fn sutra_export_ntriples(db: *const SutraDb) -> *mut c_char {
    let db = match unsafe_db_ref(db) {
        Some(d) => d,
        None => {
            set_error("Null database handle");
            return std::ptr::null_mut();
        }
    };
    let inner = match db.inner.lock() {
        Ok(i) => i,
        Err(e) => {
            set_error(&format!("Lock error: {}", e));
            return std::ptr::null_mut();
        }
    };

    let mut output = String::new();
    for triple in inner.store.iter() {
        let s = resolve_id(triple.subject, &inner.dict);
        let p = resolve_id(triple.predicate, &inner.dict);
        let o = resolve_id(triple.object, &inner.dict);
        output.push_str(&format!("{} {} {} .\n", s, p, o));
    }
    string_to_c(&output)
}

// ─── String memory management ────────────────────────────────────────────────

/// Free a C string returned by any `sutra_*` function.
///
/// Must be called on every non-null string returned by this library.
#[no_mangle]
pub extern "C" fn sutra_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}

// ─── Version ─────────────────────────────────────────────────────────────────

/// Get the SutraDB version string.
///
/// Returns a static string — do NOT free it.
#[no_mangle]
pub extern "C" fn sutra_version() -> *const c_char {
    // This is a static string, no need to free
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char
}

// ─── Internal helpers ────────────────────────────────────────────────────────

fn unsafe_cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok() }
}

fn unsafe_db_ref<'a>(ptr: *const SutraDb) -> Option<&'a SutraDb> {
    if ptr.is_null() {
        None
    } else {
        unsafe { Some(&*ptr) }
    }
}

fn string_to_c(s: &str) -> *mut c_char {
    CString::new(s).map(|c| c.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn resolve_id(id: sutra_core::TermId, dict: &sutra_core::TermDictionary) -> String {
    if let Some(n) = sutra_core::decode_inline_integer(id) {
        return n.to_string();
    }
    if let Some(b) = sutra_core::decode_inline_boolean(id) {
        return b.to_string();
    }
    dict.resolve(id)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("_:id{}", id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_close() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sdb");
        let path_c = CString::new(path.to_str().unwrap()).unwrap();

        let db = sutra_db_open(path_c.as_ptr());
        assert!(!db.is_null());
        assert_eq!(sutra_triple_count(db), 0);
        sutra_db_close(db);
    }

    #[test]
    fn test_insert_and_query() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sdb");
        let path_c = CString::new(path.to_str().unwrap()).unwrap();

        let db = sutra_db_open(path_c.as_ptr());
        assert!(!db.is_null());

        let data = CString::new(
            "<http://example.org/Alice> <http://example.org/knows> <http://example.org/Bob> .",
        )
        .unwrap();
        let inserted = sutra_insert_ntriples(db, data.as_ptr());
        assert_eq!(inserted, 1);
        assert_eq!(sutra_triple_count(db), 1);

        let query = CString::new("SELECT ?s ?p ?o WHERE { ?s ?p ?o }").unwrap();
        let result = sutra_query(db, query.as_ptr());
        assert!(!result.is_null());
        assert_eq!(sutra_result_row_count(result), 1);
        assert_eq!(sutra_result_column_count(result), 3);

        sutra_result_free(result);
        sutra_db_close(db);
    }

    #[test]
    fn test_version() {
        let v = sutra_version();
        assert!(!v.is_null());
        let version = unsafe { CStr::from_ptr(v) }.to_str().unwrap();
        assert!(!version.is_empty());
    }
}
