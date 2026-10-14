#![no_std]
#![no_main]

use libc::{arg, close, open, print, println, read, write, O_RDONLY};

const STDOUT: i32 = 1;
const STDERR: i32 = 2;
const MAX_NODES: usize = 32;
const MAX_NAME_LEN: usize = 16;

struct Graph {
    names: [[u8; MAX_NAME_LEN]; MAX_NODES],
    name_lens: [usize; MAX_NODES],
    node_count: usize,
    adj: [[bool; MAX_NODES]; MAX_NODES],
    in_degree: [usize; MAX_NODES],
}

impl Graph {
    fn new() -> Self {
        Self {
            names: [[0u8; MAX_NAME_LEN]; MAX_NODES],
            name_lens: [0usize; MAX_NODES],
            node_count: 0,
            adj: [[false; MAX_NODES]; MAX_NODES],
            in_degree: [0usize; MAX_NODES],
        }
    }

    fn find_or_add_node(&mut self, token: &[u8]) -> Option<usize> {
        let len = core::cmp::min(token.len(), MAX_NAME_LEN);
        for i in 0..self.node_count {
            if self.name_lens[i] == len && &self.names[i][..len] == &token[..len] {
                return Some(i);
            }
        }
        if self.node_count >= MAX_NODES {
            return None;
        }
        let idx = self.node_count;
        self.names[idx][..len].copy_from_slice(&token[..len]);
        self.name_lens[idx] = len;
        self.node_count += 1;
        Some(idx)
    }

    fn add_edge(&mut self, u: usize, v: usize) {
        if u != v && !self.adj[u][v] {
            self.adj[u][v] = true;
            self.in_degree[v] += 1;
        }
    }

    fn tsort(&mut self) -> i32 {
        let mut in_deg = self.in_degree;
        let mut emitted = [false; MAX_NODES];
        let mut emitted_count = 0usize;

        while emitted_count < self.node_count {
            // Find a node with 0 in-degree that hasn't been emitted
            let mut found = None;
            for i in 0..self.node_count {
                if !emitted[i] && in_deg[i] == 0 {
                    found = Some(i);
                    break;
                }
            }

            match found {
                Some(u) => {
                    emitted[u] = true;
                    emitted_count += 1;
                    let len = self.name_lens[u];
                    let _ = write(STDOUT, &self.names[u][..len]);
                    let _ = write(STDOUT, b"\n");

                    for v in 0..self.node_count {
                        if self.adj[u][v] {
                            if in_deg[v] > 0 {
                                in_deg[v] -= 1;
                            }
                        }
                    }
                }
                None => {
                    // Cycle detected!
                    let _ = write(STDERR, b"tsort: cycle in data\n");
                    // Emit remaining nodes to allow pipeline completion
                    for i in 0..self.node_count {
                        if !emitted[i] {
                            let len = self.name_lens[i];
                            let _ = write(STDOUT, &self.names[i][..len]);
                            let _ = write(STDOUT, b"\n");
                        }
                    }
                    return 1;
                }
            }
        }
        0
    }
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    let mut fd = 0i32;

    if argc > 1 {
        let s = unsafe { arg(argv, 1) };
        if s == "-h" || s == "--help" {
            println!("Usage: tsort [FILE]");
            println!("Write totally ordered list consistent with the partial ordering in FILE");
            println!("Tokens are pairs of whitespace-separated strings (u v meaning u precedes v)");
            return 0;
        }
        if s != "-" {
            let opened = open(s, O_RDONLY);
            if opened < 0 {
                print!("tsort: cannot open ");
                println!("{}", s);
                return 1;
            }
            fd = opened as i32;
        }
    }

    let mut graph = Graph::new();
    let mut buf = [0u8; 512];
    let mut token_buf = [0u8; 32];
    let mut token_len = 0usize;
    let mut prev_node: Option<usize> = None;
    let mut pair_count = 0usize;

    loop {
        let n = read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        for &b in &buf[..n as usize] {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                if token_len > 0 {
                    if let Some(idx) = graph.find_or_add_node(&token_buf[..token_len]) {
                        match prev_node {
                            None => {
                                prev_node = Some(idx);
                            }
                            Some(u) => {
                                graph.add_edge(u, idx);
                                prev_node = None;
                                pair_count += 1;
                            }
                        }
                    }
                    token_len = 0;
                }
            } else if token_len < token_buf.len() {
                token_buf[token_len] = b;
                token_len += 1;
            }
        }
    }

    if token_len > 0 {
        if let Some(idx) = graph.find_or_add_node(&token_buf[..token_len]) {
            match prev_node {
                None => {
                    prev_node = Some(idx);
                }
                Some(u) => {
                    graph.add_edge(u, idx);
                    prev_node = None;
                    pair_count += 1;
                }
            }
        }
    }

    if fd > 0 {
        close(fd);
    }

    if prev_node.is_some() {
        let _ = write(STDERR, b"tsort: odd number of tokens\n");
        return 1;
    }

    graph.tsort()
}
