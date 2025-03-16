use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::time::{interval, Duration};
use std::process::Command;

/// Maximum number of entries allowed in the cache when memory usage is below threshold.
const DEFAULT_MAX_ENTRIES: usize=100_000;

/// Memory threshold (70% of system memory) to trigger more aggressive eviction.
const MEMORY_THRESHOLD_PERCENT: usize=70;

/// Each cache entry holds the value and a use_bit that tracks recent access.
struct CacheEntry {
    value: String,
    /// The use_bit is updated on each access.
    use_bit: AtomicBool,
    /// When the entry was last accessed (monotonic counter).
    last_access: AtomicUsize,
}

impl CacheEntry {
    #[inline]
    fn new(value: String, access_counter: usize)->Self {
        CacheEntry {
            value,
            // New entries are marked as recently used.
            use_bit: AtomicBool::new(true),
            // Set the current access counter value
            last_access: AtomicUsize::new(access_counter),
        }
    }
}

/// Our cache uses DashMap for concurrent access.
#[derive(Clone)]
struct Cache {
    map: Arc<DashMap<String, CacheEntry>>,
    access_counter: Arc<AtomicUsize>,
}

impl Cache {
    fn new()->Self {
        Cache {
            map: Arc::new(DashMap::new()),
            access_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Inserts or updates an entry.
    #[inline]
    fn put(&self, key: String, value: String) {
        // Increment the access counter for each operation
        let counter=self.access_counter.fetch_add(1, Ordering::SeqCst);
        self.map.insert(key, CacheEntry::new(value, counter));
    }

    /// Retrieves an entry by key and marks it as recently used.
    #[inline]
    fn get(&self, key: &str)->Option<String> {
        if let Some(entry)=self.map.get(key) {
            // Mark as recently used
            entry.use_bit.store(true, Ordering::Release);
            
            // Update the last access timestamp
            let counter=self.access_counter.fetch_add(1, Ordering::SeqCst);
            entry.last_access.store(counter, Ordering::Release);
            
            Some(entry.value.clone())
        } else {
            None
        }
    }

    /// Get the current memory usage percentage
    fn get_memory_usage_percent(&self)->usize {
        // Try to read memory usage from procfs on Linux
        if let Ok(output)=Command::new("sh")
            .arg("-c")
            .arg("free | grep Mem | awk '{print $3/$2 * 100}'")
            .output() 
        {
            if let Ok(output_str)=String::from_utf8(output.stdout) {
                if let Ok(value)=output_str.trim().parse::<f64>() {
                    return value as usize;
                }
            }
        }
        
        // Fallback: estimate memory usage based on cache size
        // This is a very rough approximation
        let entry_count=self.map.len();
        let avg_key_size=32; // Assume average key size of 32 bytes
        let avg_value_size=64; // Assume average value size of 64 bytes
        let overhead=32; // Overhead per entry for metadata
        
        let estimated_memory=entry_count*(avg_key_size+avg_value_size+overhead);
        
        // Assuming 2GB RAM on t3.small (use u64 to avoid overflow)
        let total_memory=2_u64*1024*1024*1024;
        
        // Calculate percentage
        ((estimated_memory as f64/total_memory as f64)*100.0) as usize
    }

    /// Calculate the dynamic maximum entries based on memory usage
    fn get_max_entries(&self)->usize {
        let memory_usage=self.get_memory_usage_percent();
        
        if memory_usage<=MEMORY_THRESHOLD_PERCENT {
            // If memory usage is below threshold, keep the default max
            DEFAULT_MAX_ENTRIES
        } else {
            // Otherwise, gradually reduce max entries as memory usage increases
            // At 100% memory usage, we'd allow only 20% of DEFAULT_MAX_ENTRIES
            let reduction_factor=(100-memory_usage) as f64/(100-MEMORY_THRESHOLD_PERCENT) as f64;
            // Ensure we don't reduce too aggressively
            let reduction_factor=f64::max(0.2, reduction_factor);
            
            (DEFAULT_MAX_ENTRIES as f64*reduction_factor) as usize
        }
    }

    /// Evicts entries using multi-tiered eviction strategy:
    /// 1. First uses clock algorithm for normal eviction
    /// 2. Falls back to LRU if needed to meet memory constraints
    fn evict(&self) {
        // Get current memory usage and determine max entries
        let memory_usage=self.get_memory_usage_percent();
        let current_max_entries=self.get_max_entries();
        
        // Skip eviction if under limits and memory is below threshold
        if self.map.len()<=current_max_entries && memory_usage<MEMORY_THRESHOLD_PERCENT {
            return;
        }
        
        // Determine how many entries need to be evicted
        let current_size=self.map.len();
        let target_size=if memory_usage>=MEMORY_THRESHOLD_PERCENT {
            // More aggressive eviction when memory pressure is high
            current_max_entries
        } else {
            // Normal eviction to stay under entry limit
            current_max_entries
        };
        
        // Skip if nothing to evict
        if current_size<=target_size {
            return;
        }
        
        let entries_to_evict=current_size-target_size;
        
        // Track entries marked for eviction in first pass
        let mut to_evict=Vec::new();
        
        // First pass - use clock algorithm: reset use bits, mark unused entries
        for entry in self.map.iter() {
            if entry.use_bit.load(Ordering::Acquire) {
                // Reset use bit
                entry.use_bit.store(false, Ordering::Release);
            } else {
                // Entry wasn't used since last cycle, mark for eviction
                to_evict.push(entry.key().clone());
            }
        }
        
        // If first pass didn't mark enough entries, do a second pass using LRU
        if to_evict.len()<entries_to_evict && memory_usage>=MEMORY_THRESHOLD_PERCENT {
            // Collect all entries with access times
            let mut lru_candidates=Vec::new();
            for entry in self.map.iter() {
                // Skip entries already marked for eviction
                if !to_evict.contains(entry.key()) {
                    lru_candidates.push((
                        entry.key().clone(),
                        entry.last_access.load(Ordering::Acquire),
                    ));
                }
            }
            
            // Sort by last access time (ascending)
            lru_candidates.sort_by_key(|&(_, timestamp)| timestamp);
            
            // Take additional entries needed
            let additional_needed=entries_to_evict-to_evict.len();
            for (key, _) in lru_candidates.iter().take(additional_needed) {
                to_evict.push(key.clone());
            }
        }
        
        // Print eviction stats for debugging
        println!(
            "Cache eviction: memory={}%, current_size={}, max_entries={}, evicting={}",
            memory_usage, current_size, current_max_entries, to_evict.len()
        );
        
        // Remove the entries marked for eviction
        for key in to_evict {
            self.map.remove(&key);
        }
    }
}

/// Request and response models for HTTP endpoints.
#[derive(Deserialize)]
struct PutRequest {
    key: String,
    value: String,
}

#[derive(Serialize)]
struct ResponseMessage {
    status: String,
    message: String,
}

#[derive(Serialize)]
struct GetResponse {
    status: String,
    key: String,
    value: String,
}

/// HTTP handler for the PUT operation.
#[post("/put")]
async fn put_handler(cache: web::Data<Cache>, req: web::Json<PutRequest>)->impl Responder {
    // Enforce maximum length for key and value (256 characters)
    if req.key.len()>256 || req.value.len()>256 {
        return HttpResponse::BadRequest().json(ResponseMessage {
            status: "ERROR".into(),
            message: "Key or Value exceeds 256 characters.".into(),
        });
    }
    
    // Check memory usage before adding new entry
    let memory_usage=cache.get_memory_usage_percent();
    if memory_usage>=95 {
        // Run emergency eviction if memory is critically high
        cache.evict();
    }
    
    cache.put(req.key.clone(), req.value.clone());
    HttpResponse::Ok().json(ResponseMessage {
        status: "OK".into(),
        message: "Key inserted/updated successfully.".into(),
    })
}

/// HTTP handler for the GET operation.
#[get("/get")]
async fn get_handler(cache: web::Data<Cache>, query: web::Query<HashMap<String, String>>)->impl Responder {
    let key=match query.get("key") {
        Some(k)=>k,
        None=> {
            return HttpResponse::BadRequest().json(ResponseMessage {
                status: "ERROR".into(),
                message: "Missing key parameter.".into(),
            })
        }
    };

    if let Some(value)=cache.get(key) {
        HttpResponse::Ok().json(GetResponse {
            status: "OK".into(),
            key: key.clone(),
            value,
        })
    } else {
        HttpResponse::NotFound().json(ResponseMessage {
            status: "ERROR".into(),
            message: "Key not found.".into(),
        })
    }
}

#[actix_web::main]
async fn main()->std::io::Result<()> {
    // Read environment variables
    let workers=std::env::var("WORKERS")
        .unwrap_or_else(|_| "2".to_string())
        .parse::<usize>()
        .unwrap_or(2);
    
    println!("Starting key-value cache service with {} workers", workers);
    
    // Initialize the shared cache.
    let cache=Cache::new();

    // Clone cache handle for the background eviction task.
    let eviction_cache=cache.clone();

    // Spawn a background task that periodically evicts entries.
    tokio::spawn(async move {
        // Set up an interval timer - check every second
        let mut interval=interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            eviction_cache.evict();
        }
    });

    // Start the Actix Web server on port 7171.
    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(cache.clone()))
            .service(put_handler)
            .service(get_handler)
    })
    .workers(workers)
    .bind("0.0.0.0:7171")?
    .run()
    .await
}