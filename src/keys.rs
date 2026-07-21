//! Hash-first key interning table (FNV-1a), matching Java `KeySymbolTable`.

const FNV_OFFSET_BASIS: u32 = 0x811c_9dc5;
const FNV_PRIME: u32 = 0x0100_0193;
const MAX_ENTRIES: usize = 10_000;
const DEFAULT_CAPACITY: usize = 64;

pub struct KeySymbolTable {
    hashes: Vec<u32>,
    keys: Vec<Option<Vec<u8>>>,
    values: Vec<Option<String>>,
    size: usize,
    mask: usize,
}

impl KeySymbolTable {
    pub fn new() -> Self {
        let mut t = Self {
            hashes: Vec::new(),
            keys: Vec::new(),
            values: Vec::new(),
            size: 0,
            mask: 0,
        };
        t.allocate(DEFAULT_CAPACITY);
        t
    }

    fn allocate(&mut self, capacity: usize) {
        self.hashes = vec![0; capacity];
        self.keys = vec![None; capacity];
        self.values = vec![None; capacity];
        self.mask = capacity - 1;
    }

    pub fn initial_hash() -> u32 {
        FNV_OFFSET_BASIS
    }

    pub fn hash_byte(hash: u32, b: u8) -> u32 {
        (hash ^ u32::from(b)).wrapping_mul(FNV_PRIME)
    }

    pub fn lookup(&self, bytes: &[u8], hash: u32) -> Option<&str> {
        let mut idx = (hash as usize) & self.mask;
        let mut probes = 0;
        while self.values[idx].is_some() {
            if self.hashes[idx] == hash {
                if let Some(ref key) = self.keys[idx] {
                    if key.as_slice() == bytes {
                        return self.values[idx].as_deref();
                    }
                }
            }
            idx = (idx + 1) & self.mask;
            probes += 1;
            if probes > self.mask {
                return None;
            }
        }
        None
    }

    pub fn put(&mut self, bytes: &[u8], hash: u32, value: String) {
        if self.size >= MAX_ENTRIES {
            return;
        }
        let cap = self.mask + 1;
        if self.size >= cap - (cap >> 2) {
            self.grow();
        }
        let mut idx = (hash as usize) & self.mask;
        while self.values[idx].is_some() {
            idx = (idx + 1) & self.mask;
        }
        self.hashes[idx] = hash;
        self.keys[idx] = Some(bytes.to_vec());
        self.values[idx] = Some(value);
        self.size += 1;
    }

    fn grow(&mut self) {
        let old_hashes = std::mem::take(&mut self.hashes);
        let old_keys = std::mem::take(&mut self.keys);
        let old_values = std::mem::take(&mut self.values);
        self.allocate((self.mask + 1) * 2);
        for i in 0..old_values.len() {
            if let Some(value) = old_values[i].clone() {
                let hash = old_hashes[i];
                let key = old_keys[i].clone().unwrap_or_default();
                let mut idx = (hash as usize) & self.mask;
                while self.values[idx].is_some() {
                    idx = (idx + 1) & self.mask;
                }
                self.hashes[idx] = hash;
                self.keys[idx] = Some(key);
                self.values[idx] = Some(value);
            }
        }
    }
}

impl Default for KeySymbolTable {
    fn default() -> Self {
        Self::new()
    }
}
