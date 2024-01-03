extern "C" {
    pub fn merkle_setroot(x: u64);
    pub fn merkle_address(x: u64);
    pub fn merkle_set(x: u64);
    pub fn merkle_get() -> u64;
    pub fn merkle_getroot() -> u64;
}

use crate::cache;
use crate::kvpair::{SMT, SMTU64};
use crate::poseidon::PoseidonHasher;
use crate::require;

pub struct Merkle {
    pub root: [u64; 4],
}

// buf to receive max size of merkle leaf data node
static mut DATA_NODE_BUF: [u64; 1024] = [0; 1024];

impl Merkle {
    /// New Merkle with initial root hash
    /// set root with move to avoid copy
    pub fn load(root: [u64; 4]) -> Self {
        Merkle { root }
    }

    pub fn new() -> Self {
        //THE following is the depth=31, 32 level merkle root default
        let root = [
            14789582351289948625,
            10919489180071018470,
            10309858136294505219,
            2839580074036780766,
        ];
        Merkle { root }
    }

    /// Get the raw leaf data of a merkle subtree
    pub fn get_simple(&mut self, index: u32, data: &mut [u64; 4]) {
        unsafe {
            merkle_address(index as u64); // build in merkle address has default depth 32

            merkle_setroot(self.root[0]);
            merkle_setroot(self.root[1]);
            merkle_setroot(self.root[2]);
            merkle_setroot(self.root[3]);

            data[0] = merkle_get();
            data[1] = merkle_get();
            data[2] = merkle_get();
            data[3] = merkle_get();

            //enforce root does not change
            merkle_getroot();
            merkle_getroot();
            merkle_getroot();
            merkle_getroot();
        }
    }

    /// Set the raw leaf data of a merkle subtree but does enforced the get/set pair convention
    pub unsafe fn set_simple_unsafe(&mut self, index: u32, data: &[u64; 4]) {
        unsafe {
            // perform the set
            merkle_address(index as u64);

            merkle_setroot(self.root[0]);
            merkle_setroot(self.root[1]);
            merkle_setroot(self.root[2]);
            merkle_setroot(self.root[3]);

            merkle_set(data[0]);
            merkle_set(data[1]);
            merkle_set(data[2]);
            merkle_set(data[3]);

            self.root[0] = merkle_getroot();
            self.root[1] = merkle_getroot();
            self.root[2] = merkle_getroot();
            self.root[3] = merkle_getroot();
        }
    }

    /// Set the raw leaf data of a merkle subtree
    pub fn set_simple(&mut self, index: u32, data: &[u64; 4], hint: Option<&[u64; 4]>) {
        // place a dummy get for merkle proof convension
        unsafe {
            merkle_address(index as u64);
            merkle_setroot(self.root[0]);
            merkle_setroot(self.root[1]);
            merkle_setroot(self.root[2]);
            merkle_setroot(self.root[3]);
        }
        if let Some(hint_data) = hint {
            unsafe {
                require(hint_data[0] == merkle_get());
                require(hint_data[1] == merkle_get());
                require(hint_data[2] == merkle_get());
                require(hint_data[3] == merkle_get());
            }
        } else {
            unsafe {
                merkle_get();
                merkle_get();
                merkle_get();
                merkle_get();
            }
        }
        unsafe {
            //enforce root does not change
            merkle_getroot();
            merkle_getroot();
            merkle_getroot();
            merkle_getroot();

            // perform the set
            self.set_simple_unsafe(index, data);
        }
    }

    pub fn get(&mut self, index: u32, data: &mut [u64], hash: &mut [u64; 4], pad: bool) -> u64 {
        self.get_simple(index, hash);
        let len = cache::fetch_data(&hash, data);
        if len > 0 {
            // FIXME: avoid copy here
            let hash_check = PoseidonHasher::hash(&data[0..len as usize], pad);
            unsafe {
                require(hash[0] == hash_check[0]);
                require(hash[1] == hash_check[1]);
                require(hash[2] == hash_check[2]);
                require(hash[3] == hash_check[3]);
            }
        } else {
            unsafe {
                require(hash[0] == 0);
                require(hash[1] == 0);
                require(hash[2] == 0);
                require(hash[3] == 0);
            }
        }
        len
    }

    /// safe version of set which enforces a get before set
    pub fn set(&mut self, index: u32, data: &[u64], pad: bool, hint: Option<&[u64; 4]>) {
        let hash = PoseidonHasher::hash(data, pad);
        cache::store_data(&hash, data);
        self.set_simple(index, &hash, hint);
    }

    /// unsafe version of set which does not enforce the get/set pair convention
    pub unsafe fn set_unsafe(&mut self, index: u32, data: &[u64], pad: bool) {
        let hash = PoseidonHasher::hash(data, pad);
        cache::store_data(&hash, data);
        self.set_simple_unsafe(index, &hash);
    }
}

const LEAF_NODE: u64 = 0;
const TREE_NODE: u64 = 1;

// internal func: key must have length 4
fn data_matches_key(data: &[u64], key: &[u64]) -> bool {
    // Recall that data[0] == LEAF_NODE
    data[1] == key[0] && data[2] == key[1] && data[3] == key[2] && data[4] == key[3]
    /*
    for i in 0..4 {
        if data[i + 1] != key[i] {
            return false;
        };
    }
    return true;
    */
}

// using a static buf to avoid memory allocation in smt implementation
fn set_smt_data(node_buf: &mut [u64], t: u64, key: &[u64], data: &[u64]) {
    node_buf[0] = t;
    node_buf[1] = key[0];
    node_buf[2] = key[1];
    node_buf[3] = key[2];
    node_buf[4] = key[3];
    for i in 0..data.len() {
        node_buf[5 + i] = data[i];
    }
}

impl Merkle {
    fn smt_get_local(&mut self, key: &[u64; 4], path_index: usize, data: &mut [u64]) -> u64 {
        //crate::dbg!("start smt_get_local {}\n", path_index);
        unsafe { require(path_index < 8) };
        let local_index = (key[path_index / 2] >> (32 * (path_index % 2))) as u32;
        let mut hash = [0; 4];
        // pad is true since the leaf might the root of a sub merkle
        let len = self.get(local_index, data, &mut hash, true);
        if len == 0 {
            // no node was find
            return 0;
        } else {
            //crate::dbg!("smt_get_local with data {:?}\n", data);
            if (data[0] & 0x1) == LEAF_NODE {
                //crate::dbg!("smt_get_local is leaf\n");
                if data_matches_key(data, key) {
                    for i in 0..(len - 5) {
                        data[i as usize] = data[i as usize + 5]
                    }
                    return len - 5;
                } else {
                    // not hit and return len = 0
                    return 0;
                }
            } else {
                //crate::dbg!("smt_get_local is node: continue in sub merkle\n");
                unsafe { require((data[0] & 0x1) == TREE_NODE) };
                let mut sub_merkle = Merkle::load(data[1..5].try_into().unwrap());
                sub_merkle.smt_get_local(key, path_index + 1, data)
            }
        }
    }

    fn smt_set_local(&mut self, key: &[u64], path_index: usize, data: &[u64]) {
        unsafe { require(path_index < 8) };
        let local_index = (key[path_index / 2] >> (32 * (path_index % 2))) as u32;
        let node_buf = unsafe { DATA_NODE_BUF.as_mut_slice() };
        let mut hint_hash = [0; 4];
        let len = self.get(local_index, node_buf, &mut hint_hash, true);
        if len == 0 {
            let data_len = data.len();
            //crate::dbg!("smt set local not hit update data {}:\n", data_len);
            set_smt_data(node_buf, LEAF_NODE, key, data);
            unsafe {
                self.set_unsafe(local_index, &node_buf[0..5 + data_len], true);
            }
        } else {
            //crate::dbg!("smt set local hit:\n");
            if (node_buf[0] & 0x1) == LEAF_NODE {
                //crate::dbg!("current node for set is leaf:\n");
                if data_matches_key(node_buf, key) {
                    let data_len = data.len();
                    //crate::dbg!("key match update data {}:\n", data_len);
                    // if hit the current node
                    set_smt_data(node_buf, LEAF_NODE, key, data);
                    unsafe {
                        self.set_unsafe(local_index, &node_buf[0..5 + data_len], true);
                    }
                } else {
                    //crate::dbg!("key not match, creating sub node:\n");
                    // conflict of key here
                    // 1. start a new merkle sub tree
                    let mut sub_merkle = Merkle::new();
                    sub_merkle.smt_set_local(
                        &node_buf[1..5],
                        path_index + 1,
                        &node_buf[5..len as usize],
                    );
                    sub_merkle.smt_set_local(key, path_index + 1, data);
                    set_smt_data(node_buf, TREE_NODE, sub_merkle.root.as_slice(), &[]);
                    // 2 update the current node with the sub merkle tree
                    // OPT: shoulde be able to use the hint_hash in the future
                    self.set(local_index, &node_buf[0..5], true, None);
                }
            } else {
                //crate::dbg!("current node for set is node:\n");
                // the node is already a sub merkle
                unsafe { require((node_buf[0] & 0x1) == TREE_NODE) };
                let mut sub_merkle = Merkle::load(node_buf[1..5].try_into().unwrap());
                sub_merkle.smt_set_local(key, path_index + 1, data);
                set_smt_data(node_buf, TREE_NODE, sub_merkle.root.as_slice(), &[]);
                self.set(local_index, &node_buf[0..5], true, None);
            }
        }
    }
}

impl SMT for Merkle {
    fn smt_get(&mut self, key: &[u64; 4], data: &mut [u64]) -> u64 {
        self.smt_get_local(key, 0, data)
    }

    fn smt_set(&mut self, key: &[u64; 4], data: &[u64]) {
        self.smt_set_local(key, 0, data)
    }
}

const IS_NODE_BIT: u64 = 0b1000000 << 56;
const IS_EMPTY_BIT: u64 = 0b100000 << 56;

fn is_leaf(a: u64) -> bool {
    (a & IS_NODE_BIT) == 0
}

fn is_empty(a: u64) -> bool {
    (a & IS_EMPTY_BIT) == 0
}



impl Merkle {
    // optimized version for
    fn smt_get_local_u64(&mut self, key: u64, path_index: usize) -> u64 {
        //crate::dbg!("start smt_get_local {}\n", path_index);
        unsafe { require(path_index < 2) };
        let local_index = (key >> (32 * (path_index % 2))) as u32;
        // pad is true since the leaf might the root of a sub merkle
        let mut stored_data = [0; 4];
        self.get_simple(local_index, &mut stored_data);
        // data is stored in little endian
        let is_leaf = is_leaf(stored_data[3]);
        if is_leaf {
            // second highest bit indicates the leaf node is empty or not
            let is_empty = is_empty(stored_data[3]);
            let stored_key = stored_data[0];
            if (!is_empty) && key == stored_key {
                return stored_data[1];
            } else {
                // is empty or not hit
                return 0;
            }
        } else {
            // make sure that there are only 2 level
            unsafe {
                crate::require(path_index == 0);
            }
            crate::dbg!("smt_get_local is node: continue in sub merkle\n");
            stored_data[3] = stored_data[3] & !IS_NODE_BIT;
            let mut sub_merkle = Merkle::load(stored_data);
            sub_merkle.smt_get_local_u64(key, path_index + 1)
        }
    }

    fn smt_set_local_u64(&mut self, key: u64, path_index: usize, data: u64) {
        unsafe { require(path_index < 2) };
        let local_index = (key >> (32 * path_index)) as u32;
        let mut stored_data = [0; 4];
        self.get_simple(local_index, &mut stored_data);
        let is_leaf = is_leaf(stored_data[3]);

        // LEAF_NODE must equal zero
        if is_leaf {
            let is_empty = is_empty(stored_data[3]);
            if is_empty {
                self.set_simple(local_index, &[key, data, 0, IS_EMPTY_BIT], None);
            } else {
                crate::dbg!("smt set local hit:\n");
                if key == stored_data[0] {
                    crate::dbg!("current node for set is leaf:\n");
                    stored_data[0] = key;
                    stored_data[1] = data;
                    self.set_simple(local_index, &stored_data, None);
                } else {
                    crate::dbg!("key not match, creating sub node:\n");
                    // conflict of key here
                    // 1. start a new merkle sub tree
                    let mut sub_merkle = Merkle::new();
                    sub_merkle.smt_set_local_u64(stored_data[0], path_index + 1, stored_data[1]);
                    sub_merkle.smt_set_local_u64(key, path_index + 1, data);
                    stored_data = sub_merkle.root;
                    stored_data[3] = stored_data[3] | IS_NODE_BIT;
                    crate::dbg!("hash is {:?}\n", stored_data);
                    // 2 update the current node with the sub merkle tree
                    self.set_simple(local_index, &stored_data, None);
                }
            }
        } else {
            crate::dbg!("current node for set is node:\n");
            // the node is already a sub merkle
            // make sure that there are only 2 level
            unsafe {
                crate::require(path_index == 0);
            }
            stored_data[3] = stored_data[3] & !IS_NODE_BIT;
            crate::dbg!("fetch hash is {:?}\n", stored_data);
            let mut sub_merkle = Merkle::load(stored_data);
            sub_merkle.smt_set_local_u64(key, path_index + 1, data);
            sub_merkle.root[3] = sub_merkle.root[3] | IS_NODE_BIT;
            self.set_simple(local_index, &sub_merkle.root, None);
        }
    }
}

impl SMTU64 for Merkle {
    fn smt_get(&mut self, key: u64) -> u64 {
        self.smt_get_local_u64(key, 0)
    }

    fn smt_set(&mut self, key: u64, data: u64) {
        self.smt_set_local_u64(key, 0, data)
    }
}
