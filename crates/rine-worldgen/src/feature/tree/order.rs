//! `java.util.HashSet<BlockPos>` iteration order, reproduced by porting `java.util.HashMap`'s bin code.
//!
//! Tree decorators and `updateLeaves` iterate their position sets, and `Util.shuffle` / stable sorts depend
//! on that order, so the RNG draws of a tree follow Java's hash layout. This module keeps the same node
//! graph as `HashMap`: plain singly linked bins, `TreeNode` bins (`next`/`prev` list plus a red-black tree
//! over `left`/`right`/`parent`), `resize` with lo/hi splitting, `treeifyBin`, `untreeify`, `removeTreeNode`
//! and `moveRootToFront`. Each function follows the JDK 25 source line by line.
//!
//! `BlockPos` is not `Comparable<BlockPos>` (it implements `Comparable<Vec3i>`), so `comparableClassFor`
//! returns null and two different positions with an equal `hashCode` are ordered by
//! `System.identityHashCode`. That value is not reproducible across runs. Such ties are resolved as
//! "new key goes left" and counted in [`TIE_BREAKS`]; a non-zero count means the golden may differ there.

use std::hash::{BuildHasherDefault, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

pub type Pos = (i32, i32, i32);

const NIL: u32 = u32::MAX;
const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: u32 = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// Bins converted to red-black trees (`treeifyBin` with table >= 64). Diagnostic only.
pub static TREEIFY_EVENTS: AtomicUsize = AtomicUsize::new(0);
/// Equal-hash, non-equal-key comparisons decided by `System.identityHashCode` in Java. Diagnostic only.
pub static TIE_BREAKS: AtomicUsize = AtomicUsize::new(0);

/// Multiplicative hasher for small integer keys (positions and palette ids).
#[derive(Default)]
pub struct FastHasher(u64);

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.0 = (self.0.rotate_left(5) ^ i as u32 as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.0 = (self.0.rotate_left(5) ^ i as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub type FastMap<K, V> = std::collections::HashMap<K, V, BuildHasherDefault<FastHasher>>;

/// `BlockPos.hashCode` (`Vec3i.hashCode`) after `HashMap.hash` spreading.
#[inline]
pub fn java_spread_hash(p: Pos) -> u32 {
    let h = (p.1.wrapping_add(p.2.wrapping_mul(31))).wrapping_mul(31).wrapping_add(p.0) as u32;
    h ^ (h >> 16)
}

#[derive(Clone, Copy)]
struct Node {
    hash: u32,
    pos: Pos,
    next: u32,
    prev: u32,
    left: u32,
    right: u32,
    parent: u32,
    red: bool,
    /// `instanceof TreeNode`: set for every node of a bin that is currently a tree bin.
    tree: bool,
}

impl Node {
    fn new(hash: u32, pos: Pos) -> Self {
        Node { hash, pos, next: NIL, prev: NIL, left: NIL, right: NIL, parent: NIL, red: false, tree: false }
    }
}

/// A `HashSet<BlockPos>` with `java.util.HashMap` bin structure.
pub struct JavaSet {
    /// Bin heads of the current table (empty until the first insert, like Java's lazy table).
    tab: Vec<u32>,
    nodes: Vec<Node>,
    free: Vec<u32>,
    /// Key -> node index. Only an accelerator for `contains`/`remove`; the bin layout is the Java one.
    index: FastMap<Pos, u32>,
}

impl Default for JavaSet {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaSet {
    pub fn new() -> Self {
        JavaSet { tab: Vec::new(), nodes: Vec::new(), free: Vec::new(), index: FastMap::default() }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    #[inline]
    pub fn contains(&self, p: &Pos) -> bool {
        self.index.contains_key(p)
    }

    #[inline]
    fn g(&self, i: u32) -> &Node {
        &self.nodes[i as usize]
    }

    #[inline]
    fn m(&mut self, i: u32) -> &mut Node {
        &mut self.nodes[i as usize]
    }

    fn alloc(&mut self, hash: u32, pos: Pos) -> u32 {
        let node = Node::new(hash, pos);
        match self.free.pop() {
            Some(i) => {
                self.nodes[i as usize] = node;
                i
            }
            None => {
                self.nodes.push(node);
                (self.nodes.len() - 1) as u32
            }
        }
    }

    fn root_of(&self, mut x: u32) -> u32 {
        while self.g(x).parent != NIL {
            x = self.g(x).parent;
        }
        x
    }

    /// `TreeNode.moveRootToFront`.
    fn move_root_to_front(&mut self, root: u32) {
        if root == NIL || self.tab.is_empty() {
            return;
        }
        let n = self.tab.len();
        let index = (n - 1) & self.g(root).hash as usize;
        let first = self.tab[index];
        if root != first {
            self.tab[index] = root;
            let rp = self.g(root).prev;
            let rn = self.g(root).next;
            if rn != NIL {
                self.m(rn).prev = rp;
            }
            if rp != NIL {
                self.m(rp).next = rn;
            }
            if first != NIL {
                self.m(first).prev = root;
            }
            self.m(root).next = first;
            self.m(root).prev = NIL;
        }
    }

    fn rotate_left(&mut self, root: u32, p: u32) -> u32 {
        let mut root = root;
        if p == NIL {
            return root;
        }
        let r = self.g(p).right;
        if r == NIL {
            return root;
        }
        let rl = self.g(r).left;
        self.m(p).right = rl;
        if rl != NIL {
            self.m(rl).parent = p;
        }
        let pp = self.g(p).parent;
        self.m(r).parent = pp;
        if pp == NIL {
            root = r;
            self.m(r).red = false;
        } else if self.g(pp).left == p {
            self.m(pp).left = r;
        } else {
            self.m(pp).right = r;
        }
        self.m(r).left = p;
        self.m(p).parent = r;
        root
    }

    fn rotate_right(&mut self, root: u32, p: u32) -> u32 {
        let mut root = root;
        if p == NIL {
            return root;
        }
        let l = self.g(p).left;
        if l == NIL {
            return root;
        }
        let lr = self.g(l).right;
        self.m(p).left = lr;
        if lr != NIL {
            self.m(lr).parent = p;
        }
        let pp = self.g(p).parent;
        self.m(l).parent = pp;
        if pp == NIL {
            root = l;
            self.m(l).red = false;
        } else if self.g(pp).right == p {
            self.m(pp).right = l;
        } else {
            self.m(pp).left = l;
        }
        self.m(l).right = p;
        self.m(p).parent = l;
        root
    }

    fn is_red(&self, i: u32) -> bool {
        i != NIL && self.g(i).red
    }

    /// `TreeNode.balanceInsertion`.
    fn balance_insertion(&mut self, root: u32, x: u32) -> u32 {
        let mut root = root;
        let mut x = x;
        self.m(x).red = true;
        loop {
            let mut xp = self.g(x).parent;
            if xp == NIL {
                self.m(x).red = false;
                return x;
            }
            if !self.g(xp).red {
                return root;
            }
            let mut xpp = self.g(xp).parent;
            if xpp == NIL {
                return root;
            }
            let xppl = self.g(xpp).left;
            if xp == xppl {
                let xppr = self.g(xpp).right;
                if xppr != NIL && self.g(xppr).red {
                    self.m(xppr).red = false;
                    self.m(xp).red = false;
                    self.m(xpp).red = true;
                    x = xpp;
                } else {
                    if x == self.g(xp).right {
                        x = xp;
                        root = self.rotate_left(root, x);
                        xp = self.g(x).parent;
                        xpp = if xp == NIL { NIL } else { self.g(xp).parent };
                    }
                    if xp != NIL {
                        self.m(xp).red = false;
                        if xpp != NIL {
                            self.m(xpp).red = true;
                            root = self.rotate_right(root, xpp);
                        }
                    }
                }
            } else {
                if xppl != NIL && self.g(xppl).red {
                    self.m(xppl).red = false;
                    self.m(xp).red = false;
                    self.m(xpp).red = true;
                    x = xpp;
                } else {
                    if x == self.g(xp).left {
                        x = xp;
                        root = self.rotate_right(root, x);
                        xp = self.g(x).parent;
                        xpp = if xp == NIL { NIL } else { self.g(xp).parent };
                    }
                    if xp != NIL {
                        self.m(xp).red = false;
                        if xpp != NIL {
                            self.m(xpp).red = true;
                            root = self.rotate_left(root, xpp);
                        }
                    }
                }
            }
        }
    }

    /// `TreeNode.balanceDeletion`.
    fn balance_deletion(&mut self, root: u32, x: u32) -> u32 {
        let mut root = root;
        let mut x = x;
        loop {
            if x == NIL || x == root {
                return root;
            }
            let mut xp = self.g(x).parent;
            if xp == NIL {
                self.m(x).red = false;
                return x;
            }
            if self.g(x).red {
                self.m(x).red = false;
                return root;
            }
            let xpl = self.g(xp).left;
            if xpl == x {
                let mut xpr = self.g(xp).right;
                if xpr != NIL && self.g(xpr).red {
                    self.m(xpr).red = false;
                    self.m(xp).red = true;
                    root = self.rotate_left(root, xp);
                    xp = self.g(x).parent;
                    xpr = if xp == NIL { NIL } else { self.g(xp).right };
                }
                if xpr == NIL {
                    x = xp;
                } else {
                    let sl = self.g(xpr).left;
                    let sr = self.g(xpr).right;
                    if !self.is_red(sr) && !self.is_red(sl) {
                        self.m(xpr).red = true;
                        x = xp;
                    } else {
                        if !self.is_red(sr) {
                            if sl != NIL {
                                self.m(sl).red = false;
                            }
                            self.m(xpr).red = true;
                            root = self.rotate_right(root, xpr);
                            xp = self.g(x).parent;
                            xpr = if xp == NIL { NIL } else { self.g(xp).right };
                        }
                        if xpr != NIL {
                            let c = if xp == NIL { false } else { self.g(xp).red };
                            self.m(xpr).red = c;
                            let sr2 = self.g(xpr).right;
                            if sr2 != NIL {
                                self.m(sr2).red = false;
                            }
                        }
                        if xp != NIL {
                            self.m(xp).red = false;
                            root = self.rotate_left(root, xp);
                        }
                        x = root;
                    }
                }
            } else {
                let mut xpl = xpl;
                if xpl != NIL && self.g(xpl).red {
                    self.m(xpl).red = false;
                    self.m(xp).red = true;
                    root = self.rotate_right(root, xp);
                    xp = self.g(x).parent;
                    xpl = if xp == NIL { NIL } else { self.g(xp).left };
                }
                if xpl == NIL {
                    x = xp;
                } else {
                    let sl = self.g(xpl).left;
                    let sr = self.g(xpl).right;
                    if !self.is_red(sl) && !self.is_red(sr) {
                        self.m(xpl).red = true;
                        x = xp;
                    } else {
                        if !self.is_red(sl) {
                            if sr != NIL {
                                self.m(sr).red = false;
                            }
                            self.m(xpl).red = true;
                            root = self.rotate_left(root, xpl);
                            xp = self.g(x).parent;
                            xpl = if xp == NIL { NIL } else { self.g(xp).left };
                        }
                        if xpl != NIL {
                            let c = if xp == NIL { false } else { self.g(xp).red };
                            self.m(xpl).red = c;
                            let sl2 = self.g(xpl).left;
                            if sl2 != NIL {
                                self.m(sl2).red = false;
                            }
                        }
                        if xp != NIL {
                            self.m(xp).red = false;
                            root = self.rotate_right(root, xp);
                        }
                        x = root;
                    }
                }
            }
        }
    }

    /// `TreeNode.find` (keys are not comparable, so the tie branch searches the right subtree, then the left).
    fn tree_find(&self, start: u32, h: u32, k: Pos) -> u32 {
        let mut p = start;
        loop {
            let (ph, pl, pr, pk) = {
                let n = self.g(p);
                (n.hash, n.left, n.right, n.pos)
            };
            if (ph as i32) > (h as i32) {
                p = pl;
            } else if (ph as i32) < (h as i32) {
                p = pr;
            } else if pk == k {
                return p;
            } else if pl == NIL {
                p = pr;
            } else if pr == NIL {
                p = pl;
            } else {
                let q = self.tree_find(pr, h, k);
                if q != NIL {
                    return q;
                }
                p = pl;
            }
            if p == NIL {
                return NIL;
            }
        }
    }

    /// `TreeNode.treeify`: rebuild the red-black tree from the `next` list, then move the root to the front.
    fn treeify(&mut self, head: u32) {
        let mut root = NIL;
        let mut x = head;
        while x != NIL {
            let next = self.g(x).next;
            {
                let n = self.m(x);
                n.left = NIL;
                n.right = NIL;
            }
            if root == NIL {
                let n = self.m(x);
                n.parent = NIL;
                n.red = false;
                root = x;
            } else {
                let k = self.g(x).pos;
                let h = self.g(x).hash;
                let mut p = root;
                loop {
                    let ph = self.g(p).hash;
                    let dir: i32 = if (ph as i32) > (h as i32) {
                        -1
                    } else if (ph as i32) < (h as i32) {
                        1
                    } else {
                        tie_break(k, self.g(p).pos)
                    };
                    let xp = p;
                    p = if dir <= 0 { self.g(p).left } else { self.g(p).right };
                    if p == NIL {
                        self.m(x).parent = xp;
                        if dir <= 0 {
                            self.m(xp).left = x;
                        } else {
                            self.m(xp).right = x;
                        }
                        root = self.balance_insertion(root, x);
                        break;
                    }
                }
            }
            x = next;
        }
        self.move_root_to_front(root);
    }

    /// `TreeNode.untreeify`: the bin becomes a plain list in `next` order.
    fn untreeify(&mut self, head: u32) -> u32 {
        let mut q = head;
        while q != NIL {
            let n = self.m(q);
            n.tree = false;
            n.left = NIL;
            n.right = NIL;
            n.parent = NIL;
            n.red = false;
            q = self.g(q).next;
        }
        head
    }

    /// `TreeNode.putTreeVal` for a key that is known to be absent. `x` is an allocated plain node.
    /// The tie search of Java only returns existing matches, so it never changes the structure and is skipped.
    fn put_tree_val(&mut self, head: u32, x: u32) {
        let h = self.g(x).hash;
        let k = self.g(x).pos;
        let root = if self.g(head).parent != NIL { self.root_of(head) } else { head };
        let mut p = root;
        loop {
            let ph = self.g(p).hash;
            let dir: i32 = if (ph as i32) > (h as i32) {
                -1
            } else if (ph as i32) < (h as i32) {
                1
            } else {
                tie_break(k, self.g(p).pos)
            };
            let xp = p;
            let child = if dir <= 0 { self.g(p).left } else { self.g(p).right };
            if child == NIL {
                let xpn = self.g(xp).next;
                {
                    let n = self.m(x);
                    n.tree = true;
                    n.next = xpn;
                    n.parent = xp;
                    n.prev = xp;
                }
                if dir <= 0 {
                    self.m(xp).left = x;
                } else {
                    self.m(xp).right = x;
                }
                self.m(xp).next = x;
                if xpn != NIL {
                    self.m(xpn).prev = x;
                }
                let new_root = self.balance_insertion(root, x);
                self.move_root_to_front(new_root);
                return;
            }
            p = child;
        }
    }

    /// `HashMap.treeifyBin` for a bin that just reached `TREEIFY_THRESHOLD` nodes.
    /// Below table size 64 the table doubles instead; from 64 the bin becomes a red-black tree.
    fn treeify_bin(&mut self, hash: u32) {
        if self.tab.len() < MIN_TREEIFY_CAPACITY {
            self.resize();
            return;
        }
        let index = (self.tab.len() - 1) & hash as usize;
        let hd = self.tab[index];
        if hd == NIL {
            return;
        }
        TREEIFY_EVENTS.fetch_add(1, Ordering::Relaxed);
        let mut prev = NIL;
        let mut cur = hd;
        while cur != NIL {
            let next = self.g(cur).next;
            {
                let n = self.m(cur);
                n.tree = true;
                n.left = NIL;
                n.right = NIL;
                n.parent = NIL;
                n.red = false;
                n.prev = prev;
            }
            if prev != NIL {
                self.m(prev).next = cur;
            }
            prev = cur;
            cur = next;
        }
        self.treeify(hd);
    }

    /// `TreeNode.split` on the old bin `head`, splitting into `index` and `index + bit` of the new table.
    fn split(&mut self, head: u32, index: usize, bit: u32) {
        let (mut lo_h, mut lo_t, mut hi_h, mut hi_t) = (NIL, NIL, NIL, NIL);
        let (mut lc, mut hc) = (0u32, 0u32);
        let mut e = head;
        while e != NIL {
            let next = self.g(e).next;
            self.m(e).next = NIL;
            if self.g(e).hash & bit == 0 {
                self.m(e).prev = lo_t;
                if lo_t == NIL {
                    lo_h = e;
                } else {
                    self.m(lo_t).next = e;
                }
                lo_t = e;
                lc += 1;
            } else {
                self.m(e).prev = hi_t;
                if hi_t == NIL {
                    hi_h = e;
                } else {
                    self.m(hi_t).next = e;
                }
                hi_t = e;
                hc += 1;
            }
            e = next;
        }
        if lo_h != NIL {
            if lc <= UNTREEIFY_THRESHOLD {
                let u = self.untreeify(lo_h);
                self.tab[index] = u;
            } else {
                self.tab[index] = lo_h;
                if hi_h != NIL {
                    self.treeify(lo_h);
                }
            }
        }
        if hi_h != NIL {
            if hc <= UNTREEIFY_THRESHOLD {
                let u = self.untreeify(hi_h);
                self.tab[index + bit as usize] = u;
            } else {
                self.tab[index + bit as usize] = hi_h;
                if lo_h != NIL {
                    self.treeify(hi_h);
                }
            }
        }
    }

    /// `HashMap.resize`: doubles the table (16 on first use) and redistributes every bin in order.
    fn resize(&mut self) {
        let old_cap = self.tab.len();
        if old_cap == 0 {
            self.tab = vec![NIL; 16];
            return;
        }
        let new_cap = old_cap * 2;
        let old = std::mem::replace(&mut self.tab, vec![NIL; new_cap]);
        for (j, &e) in old.iter().enumerate() {
            if e == NIL {
                continue;
            }
            if self.g(e).next == NIL {
                let idx = self.g(e).hash as usize & (new_cap - 1);
                self.tab[idx] = e;
            } else if self.g(e).tree {
                self.split(e, j, old_cap as u32);
            } else {
                let (mut lo_h, mut lo_t, mut hi_h, mut hi_t) = (NIL, NIL, NIL, NIL);
                let mut cur = e;
                while cur != NIL {
                    let next = self.g(cur).next;
                    if self.g(cur).hash as usize & old_cap == 0 {
                        if lo_t == NIL {
                            lo_h = cur;
                        } else {
                            self.m(lo_t).next = cur;
                        }
                        lo_t = cur;
                    } else {
                        if hi_t == NIL {
                            hi_h = cur;
                        } else {
                            self.m(hi_t).next = cur;
                        }
                        hi_t = cur;
                    }
                    cur = next;
                }
                if lo_t != NIL {
                    self.m(lo_t).next = NIL;
                    self.tab[j] = lo_h;
                }
                if hi_t != NIL {
                    self.m(hi_t).next = NIL;
                    self.tab[j + old_cap] = hi_h;
                }
            }
        }
    }

    /// `HashMap.putVal` for a key that is known to be absent. Returns true (always inserts).
    pub fn insert(&mut self, p: Pos) -> bool {
        if self.index.contains_key(&p) {
            return false;
        }
        if self.tab.is_empty() {
            self.resize();
        }
        let h = java_spread_hash(p);
        let n = self.tab.len();
        let i = h as usize & (n - 1);
        let node = self.alloc(h, p);
        self.index.insert(p, node);
        let head = self.tab[i];
        if head == NIL {
            self.tab[i] = node;
        } else if self.g(head).tree {
            self.put_tree_val(head, node);
        } else {
            let mut bin_count = 0usize;
            let mut cur = head;
            loop {
                let next = self.g(cur).next;
                if next == NIL {
                    self.m(cur).next = node;
                    if bin_count >= TREEIFY_THRESHOLD - 1 {
                        self.treeify_bin(h);
                    }
                    break;
                }
                cur = next;
                bin_count += 1;
            }
        }
        if self.index.len() > self.tab.len() * 3 / 4 {
            self.resize();
        }
        true
    }

    /// `HashMap.removeNode` with `movable` as in `HashSet.remove` (true) or `iterator().remove()` (false).
    fn remove_node(&mut self, node: u32, movable: bool) {
        let n = self.tab.len();
        let index = (n - 1) & self.g(node).hash as usize;
        let head = self.tab[index];
        if self.g(node).tree {
            self.remove_tree_node(node, movable);
        } else if node == head {
            self.tab[index] = self.g(node).next;
        } else {
            let mut pred = head;
            while self.g(pred).next != node {
                pred = self.g(pred).next;
            }
            let nx = self.g(node).next;
            self.m(pred).next = nx;
        }
        let pos = self.g(node).pos;
        self.index.remove(&pos);
        self.free.push(node);
    }

    /// `HashSet.remove`.
    pub fn remove(&mut self, p: &Pos) -> bool {
        match self.index.get(p).copied() {
            Some(node) => {
                self.remove_node(node, true);
                true
            }
            None => false,
        }
    }

    /// `TreeNode.removeTreeNode`.
    fn remove_tree_node(&mut self, node: u32, movable: bool) {
        let n = self.tab.len();
        let index = (n - 1) & self.g(node).hash as usize;
        let mut first = self.tab[index];
        let mut root = first;
        let succ = self.g(node).next;
        let pred = self.g(node).prev;
        if pred == NIL {
            self.tab[index] = succ;
            first = succ;
        } else {
            self.m(pred).next = succ;
        }
        if succ != NIL {
            self.m(succ).prev = pred;
        }
        if first == NIL {
            return;
        }
        if self.g(root).parent != NIL {
            root = self.root_of(root);
        }
        let too_small = root == NIL || {
            movable && {
                let r = self.g(root);
                r.right == NIL || r.left == NIL || self.g(r.left).left == NIL
            }
        };
        if too_small {
            let u = self.untreeify(first);
            self.tab[index] = u;
            return;
        }
        let p = node;
        let pl = self.g(p).left;
        let pr = self.g(p).right;
        let replacement;
        if pl != NIL && pr != NIL {
            let mut s = pr;
            loop {
                let sl = self.g(s).left;
                if sl == NIL {
                    break;
                }
                s = sl;
            }
            let c = self.g(s).red;
            self.m(s).red = self.g(p).red;
            self.m(p).red = c;
            let sr = self.g(s).right;
            let pp = self.g(p).parent;
            if s == pr {
                self.m(p).parent = s;
                self.m(s).right = p;
            } else {
                let sp = self.g(s).parent;
                self.m(p).parent = sp;
                if sp != NIL {
                    if s == self.g(sp).left {
                        self.m(sp).left = p;
                    } else {
                        self.m(sp).right = p;
                    }
                }
                self.m(s).right = pr;
                if pr != NIL {
                    self.m(pr).parent = s;
                }
            }
            self.m(p).left = NIL;
            self.m(p).right = sr;
            if sr != NIL {
                self.m(sr).parent = p;
            }
            self.m(s).left = pl;
            if pl != NIL {
                self.m(pl).parent = s;
            }
            self.m(s).parent = pp;
            if pp == NIL {
                root = s;
            } else if p == self.g(pp).left {
                self.m(pp).left = s;
            } else {
                self.m(pp).right = s;
            }
            replacement = if sr != NIL { sr } else { p };
        } else if pl != NIL {
            replacement = pl;
        } else if pr != NIL {
            replacement = pr;
        } else {
            replacement = p;
        }
        if replacement != p {
            let pp = self.g(p).parent;
            self.m(replacement).parent = pp;
            if pp == NIL {
                root = replacement;
                self.m(replacement).red = false;
            } else if p == self.g(pp).left {
                self.m(pp).left = replacement;
            } else {
                self.m(pp).right = replacement;
            }
            let pm = self.m(p);
            pm.left = NIL;
            pm.right = NIL;
            pm.parent = NIL;
        }
        let r = if self.g(p).red { root } else { self.balance_deletion(root, replacement) };
        if replacement == p {
            let pp = self.g(p).parent;
            self.m(p).parent = NIL;
            if pp != NIL {
                if p == self.g(pp).left {
                    self.m(pp).left = NIL;
                } else if p == self.g(pp).right {
                    self.m(pp).right = NIL;
                }
            }
        }
        if movable {
            self.move_root_to_front(r);
        }
    }

    /// `iterator().next()` followed by `iterator.remove()`: the first element in iteration order.
    pub fn pop_first(&mut self) -> Option<Pos> {
        if self.index.is_empty() {
            return None;
        }
        let i = self.tab.iter().position(|&h| h != NIL)?;
        let head = self.tab[i];
        let pos = self.g(head).pos;
        self.remove_node(head, false);
        Some(pos)
    }

    /// Structural check of every tree bin (test diagnostics): links, colours, BST order and head == root.
    #[cfg(test)]
    pub fn check_invariants(&self) -> Result<(), String> {
        for (bi, &head) in self.tab.iter().enumerate() {
            if head == NIL || !self.g(head).tree {
                continue;
            }
            // The head is not necessarily the root: removeTreeNode(movable = false) (iterator.remove) skips
            // moveRootToFront in Java as well.
            let root = self.root_of(head);
            if self.g(root).red {
                return Err(format!("bin {bi}: red root"));
            }
            // Walk the tree, checking links and colours; collect nodes.
            let mut tree_nodes = Vec::new();
            fn walk(s: &JavaSet, n: u32, tree_nodes: &mut Vec<u32>, bi: usize) -> Result<usize, String> {
                if n == NIL {
                    return Ok(1);
                }
                tree_nodes.push(n);
                let node = *s.g(n);
                for c in [node.left, node.right] {
                    if c != NIL && s.g(c).parent != n {
                        return Err(format!("bin {bi}: child parent link broken"));
                    }
                }
                if node.red {
                    for c in [node.left, node.right] {
                        if c != NIL && s.g(c).red {
                            return Err(format!("bin {bi}: red-red"));
                        }
                    }
                }
                if node.left != NIL && (s.g(node.left).hash as i32) > (node.hash as i32) {
                    return Err(format!("bin {bi}: BST order left"));
                }
                if node.right != NIL && (s.g(node.right).hash as i32) < (node.hash as i32) {
                    return Err(format!("bin {bi}: BST order right"));
                }
                let bl = walk(s, node.left, tree_nodes, bi)?;
                let br = walk(s, node.right, tree_nodes, bi)?;
                if bl != br {
                    return Err(format!("bin {bi}: black height mismatch"));
                }
                Ok(bl + if node.red { 0 } else { 1 })
            }
            walk(self, root, &mut tree_nodes, bi)?;
            // The next list must be exactly the tree nodes, with consistent prev links.
            let mut list = Vec::new();
            let mut cur = head;
            let mut prev = NIL;
            while cur != NIL {
                if self.g(cur).prev != prev {
                    return Err(format!("bin {bi}: prev link broken"));
                }
                list.push(cur);
                prev = cur;
                cur = self.g(cur).next;
                if list.len() > 10_000 {
                    return Err(format!("bin {bi}: next cycle"));
                }
            }
            let mut a = tree_nodes.clone();
            a.sort_unstable();
            let mut b = list.clone();
            b.sort_unstable();
            if a != b {
                return Err(format!("bin {bi}: next list != tree nodes"));
            }
        }
        Ok(())
    }

    /// Elements in `HashMap` iteration order (bins by index, then each bin's `next` chain).
    pub fn to_vec(&self) -> Vec<Pos> {
        let mut out = Vec::with_capacity(self.len());
        for &head in &self.tab {
            let mut cur = head;
            while cur != NIL {
                out.push(self.g(cur).pos);
                cur = self.g(cur).next;
            }
        }
        out
    }
}

/// `TreeNode.tieBreakOrder` for two non-comparable keys with equal hashes. Java uses identity hash codes,
/// which are not reproducible, so the new key goes left and the event is counted.
#[inline]
fn tie_break(_new: Pos, _existing: Pos) -> i32 {
    TIE_BREAKS.fetch_add(1, Ordering::Relaxed);
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_set_orders_by_bucket_then_insertion() {
        let mut s = JavaSet::new();
        // hash((0,0,0)) = 0; hash((1,0,0)) = 1; hash((0,0,1)) = 31 * 31 = 961 -> bucket 961 & 15 = 1.
        s.insert((1, 0, 0));
        s.insert((0, 0, 0));
        s.insert((0, 0, 1));
        assert_eq!(s.to_vec(), vec![(0, 0, 0), (1, 0, 0), (0, 0, 1)]);
    }

    #[test]
    fn resize_keeps_bucket_order_and_pop_first_matches_iteration() {
        let mut s = JavaSet::new();
        for i in 0..13 {
            s.insert((i, 0, i % 3));
        }
        assert_eq!(s.len(), 13);
        let order = s.to_vec();
        let mut popped = Vec::new();
        while let Some(p) = s.pop_first() {
            popped.push(p);
        }
        assert_eq!(order, popped);
    }

    /// Same digest as `fnv` in tools/noise-golden/HashOrderGen.java (regenerate: `java HashOrderGen.java 1 2500 tests/data/hashset_order.txt`).
    fn jdk_fnv(order: &[Pos]) -> u64 {
        let mut h: u64 = 1469598103934665603;
        for &(x, y, z) in order {
            for v in [x, y, z] {
                h ^= v as i64 as u64;
                h = h.wrapping_mul(1099511628211);
            }
            h ^= 0x9e37_79b9_7f4a_7c15;
            h = h.wrapping_mul(1099511628211);
        }
        h ^= order.len() as u64;
        h.wrapping_mul(1099511628211)
    }

    /// Replays operations recorded from a real `java.util.HashSet` and checks the iteration digest after each one.
    #[test]
    fn matches_jdk_hashset_iteration_order() {
        let text = include_str!("../../../tests/data/hashset_order.txt");
        let treeify_before = TREEIFY_EVENTS.load(Ordering::Relaxed);
        let mut s = JavaSet::new();
        let mut checked = 0usize;
        for line in text.lines() {
            let mut parts = line.split(' ');
            let op = parts.next().unwrap();
            if op == "H" {
                let digest = u64::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let size: usize = parts.next().unwrap().parse().unwrap();
                assert_eq!(s.len(), size, "size after op {checked}");
                assert_eq!(jdk_fnv(&s.to_vec()), digest, "iteration digest after op {checked}");
                checked += 1;
                continue;
            }
            let x: i32 = parts.next().unwrap().parse().unwrap();
            let y: i32 = parts.next().unwrap().parse().unwrap();
            let z: i32 = parts.next().unwrap().parse().unwrap();
            match op {
                "A" => assert!(s.insert((x, y, z))),
                "R" => assert!(s.remove(&(x, y, z))),
                "P" => assert_eq!(s.pop_first(), Some((x, y, z))),
                other => panic!("unknown op {other}"),
            }
            s.check_invariants().unwrap();
        }
        assert!(checked > 1000);
        assert!(
            TREEIFY_EVENTS.load(Ordering::Relaxed) > treeify_before,
            "fixture must exercise tree bins"
        );
    }

    #[test]
    fn remove_and_contains_stay_consistent_with_iteration() {
        let mut s = JavaSet::new();
        for i in 0..200 {
            s.insert((i % 17, i / 17, (i * 7) % 11));
        }
        for i in (0..200).step_by(3) {
            assert!(s.remove(&(i % 17, i / 17, (i * 7) % 11)));
        }
        let order = s.to_vec();
        assert_eq!(order.len(), s.len());
        for p in &order {
            assert!(s.contains(p));
        }
        let mut popped = Vec::new();
        while let Some(p) = s.pop_first() {
            popped.push(p);
        }
        assert_eq!(order, popped);
    }
}
