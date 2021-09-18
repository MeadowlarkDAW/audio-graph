use fnv::{FnvHashMap, FnvHashSet};
use smallvec::smallvec as vec;
use smallvec::SmallVec;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::{borrow::Borrow, collections::VecDeque};

type Vec<T> = SmallVec<[T; 16]>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct NodeRef(usize);

impl From<NodeRef> for usize {
    fn from(n: NodeRef) -> Self {
        n.0
    }
}

impl Borrow<usize> for NodeRef {
    fn borrow(&self) -> &'_ usize {
        &self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct PortRef(usize);

impl Borrow<usize> for PortRef {
    fn borrow(&self) -> &'_ usize {
        &self.0
    }
}

pub trait PortType: Debug + Clone + Copy + Eq + std::hash::Hash {
    fn into_index(&self) -> usize;
    fn num_types() -> usize;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultPortType {
    Audio,
    Event,
}

impl PortType for DefaultPortType {
    #[inline]
    fn into_index(&self) -> usize {
        match self {
            DefaultPortType::Audio => 0,
            DefaultPortType::Event => 1,
        }
    }

    fn num_types() -> usize {
        2
    }
}

#[derive(Clone, Copy, Debug)]
struct Edge<PT: PortType + PartialEq> {
    src_node: NodeRef,
    src_port: PortRef,
    dst_node: NodeRef,
    dst_port: PortRef,
    type_: PT,
}

impl<PT: PortType + PartialEq> PartialEq for Edge<PT> {
    fn eq(&self, other: &Edge<PT>) -> bool {
        // For our purposes, comparing just src_port and dst_port is sufficient.
        // Ports can only be of one type, and they can only belong to one node.
        // Deleting ports should garauntee that all corresponding edges are also
        // deleted, so reusing ports should not cause a problem.
        self.src_port == other.src_port && self.dst_port == other.dst_port
    }
}

struct BufferAllocator<PT: PortType + PartialEq> {
    buffer_count_stacks: Vec<(usize, Vec<usize>)>,
    _phantom_port_type: PhantomData<PT>,
}

impl<PT: PortType + PartialEq> BufferAllocator<PT> {
    fn clear(&mut self) {
        for (c, s) in self.buffer_count_stacks.iter_mut() {
            *c = 0;
            s.clear();
        }
    }

    fn acquire(&mut self, type_: PT) -> Buffer<PT> {
        let type_index = type_.into_index();
        let (count, stack) = &mut self.buffer_count_stacks[type_index];

        if let Some(index) = stack.pop() {
            Buffer { index, type_ }
        } else {
            let buffer = Buffer {
                index: *count,
                type_,
            };
            *count += 1;
            buffer
        }
    }

    fn release(&mut self, ref_: Buffer<PT>) {
        let type_index = ref_.type_.into_index();
        let stack = &mut self.buffer_count_stacks[type_index].1;
        stack.push(ref_.index);
    }
}

impl<PT: PortType> Default for BufferAllocator<PT> {
    fn default() -> Self {
        let num_types = PT::num_types();
        let mut buffer_count_stacks = Vec::<(usize, Vec<usize>)>::new();
        for _ in 0..num_types {
            buffer_count_stacks.push((0, Vec::new()));
        }
        Self {
            buffer_count_stacks,
            _phantom_port_type: PhantomData::default(),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Buffer<PT: PortType + PartialEq> {
    index: usize,
    type_: PT,
}

#[derive(Clone, Debug)]
pub struct Scheduled<N, P, PT>
where
    N: Debug + Clone,
    P: Debug + Clone,
    PT: PortType + PartialEq,
{
    pub node: N,
    pub inputs: Vec<(P, Vec<(Buffer<PT>, u64)>)>,
    pub outputs: Vec<(P, Buffer<PT>)>,
}

pub struct HeapStore<N, P, PT>
where
    N: Debug + Clone,
    P: Debug + Clone,
    PT: PortType + PartialEq,
{
    walk_queue: Option<VecDeque<NodeRef>>,
    walk_indegree: Option<FnvHashMap<NodeRef, usize>>,
    cycle_queued: Option<FnvHashSet<NodeRef>>,

    latencies: Vec<u64>,
    all_latencies: Vec<Option<u64>>,
    deps: Vec<NodeRef>,
    allocator: BufferAllocator<PT>,
    delay_comps: Option<FnvHashMap<(PortRef, PortRef), u64>>,
    input_assignments: FnvHashMap<(NodeRef, PortRef), Vec<(Buffer<PT>, (PortRef, PortRef))>>,
    output_assignments: FnvHashMap<(NodeRef, PortRef), (Buffer<PT>, usize)>,
    scheduled_nodes: Option<Vec<NodeRef>>,

    scheduled: Option<Vec<Scheduled<N, P, PT>>>,
}

impl<N, P, PT> Default for HeapStore<N, P, PT>
where
    N: Debug + Clone,
    P: Debug + Clone,
    PT: PortType + PartialEq,
{
    fn default() -> Self {
        Self {
            walk_queue: Some(VecDeque::new()),
            walk_indegree: Some(FnvHashMap::default()),
            cycle_queued: Some(FnvHashSet::default()),
            latencies: Vec::new(),
            all_latencies: Vec::new(),
            deps: Vec::new(),
            allocator: BufferAllocator::default(),
            delay_comps: Some(FnvHashMap::default()),
            input_assignments: FnvHashMap::default(),
            output_assignments: FnvHashMap::default(),
            scheduled_nodes: Some(Vec::new()),
            scheduled: None,
        }
    }
}

pub struct Graph<N, P, PT>
where
    N: Debug + Clone,
    P: Debug + Clone,
    PT: PortType + PartialEq,
{
    edges: Vec<Vec<Edge<PT>>>,
    ports: Vec<Vec<PortRef>>,
    delays: Vec<u64>,
    port_data: Vec<(NodeRef, PT)>,

    port_identifiers: Vec<P>,
    node_identifiers: Vec<N>,
    free_nodes: Vec<NodeRef>,
    free_ports: Vec<PortRef>,

    heap_store: Option<HeapStore<N, P, PT>>,
}

impl<N, P, PT> Default for Graph<N, P, PT>
where
    N: Debug + Clone,
    P: Debug + Clone,
    PT: PortType + PartialEq,
{
    fn default() -> Self {
        Self {
            edges: Vec::new(),
            ports: Vec::new(),
            delays: Vec::new(),
            port_data: Vec::new(),
            port_identifiers: Vec::new(),
            node_identifiers: Vec::new(),
            free_nodes: Vec::new(),
            free_ports: Vec::new(),
            heap_store: Some(HeapStore::default()),
        }
    }
}

impl<N, P, PT> Graph<N, P, PT>
where
    N: Debug + Clone,
    P: Debug + Clone,
    PT: PortType + PartialEq,
{
    pub fn node(&mut self, ident: N) -> NodeRef {
        if let Some(node) = self.free_nodes.pop() {
            let id = node.0;
            self.edges[id].clear();
            self.ports[id].clear();
            self.delays[id] = 0;
            self.node_identifiers[id] = ident;
            node
        } else {
            let id = self.node_count();
            self.edges.push(vec![]);
            self.ports.push(vec![]);
            self.delays.push(0);
            self.node_identifiers.push(ident);
            NodeRef(id)
        }
    }

    pub fn port(&mut self, node: NodeRef, type_: PT, ident: P) -> Result<PortRef, Error> {
        if node.0 < self.node_count() && !self.free_nodes.contains(&node) {
            if let Some(port) = self.free_ports.pop() {
                self.ports[node.0].push(port);
                self.port_data[port.0] = (node, type_);
                self.port_identifiers[port.0] = ident;
                Ok(port)
            } else {
                let port = PortRef(self.port_count());

                self.ports[node.0].push(port);
                self.port_data.push((node, type_));
                self.port_identifiers.push(ident);

                Ok(port)
            }
        } else {
            Err(Error::NodeDoesNotExist)
        }
    }

    pub fn delete_port(&mut self, p: PortRef) -> Result<(), Error> {
        self.port_check(p)?;
        let (node, _) = self.port_data[p.0];
        for e in self.edges[node.0]
            .clone()
            .into_iter()
            .filter(|e| e.dst_port == p || e.src_port == p)
        {
            let _e = self.remove_edge(e);
            debug_assert!(_e.is_ok());
        }
        let index = self.ports[node.0]
            .iter()
            .position(|p_| *p_ == p)
            .ok_or(Error::PortDoesNotExist)?;
        self.ports[node.0].remove(index);
        self.free_ports.push(p);
        Ok(())
    }

    pub fn delete_node(&mut self, n: NodeRef) -> Result<(), Error> {
        self.node_check(n)?;
        for p in self.ports[n.0].clone() {
            let _e = self.delete_port(p);
            debug_assert!(_e.is_ok());
        }
        self.free_nodes.push(n);
        Ok(())
    }

    pub fn connect(&mut self, src: PortRef, dst: PortRef) -> Result<(), Error> {
        self.port_check(src)?;
        self.port_check(dst)?;

        let (src_node, src_type) = self.port_data[src.0];
        let (dst_node, dst_type) = self.port_data[dst.0];
        if src_type != dst_type {
            return Err(Error::InvalidPortType);
        }

        for edge in self.incoming(dst) {
            if edge.src_port == src {
                // These two ports are already connected.
                return Ok(());
            }
        }

        self.cycle_check(src_node, dst_node)?;

        let edge = Edge {
            src_node,
            src_port: src,
            dst_node,
            dst_port: dst,
            type_: src_type,
        };

        /* TODO: Maybe use the log crate for this to avoid polluting the user's output?
        println!(
            "connection {}.{} to {}.{} with edge: {:?}",
            self.node_name(src_node).unwrap(),
            self.port_name(src).unwrap(),
            self.node_name(dst_node).unwrap(),
            self.port_name(dst).unwrap(),
            edge
        );
        */

        self.edges[src_node.0].push(edge);
        self.edges[dst_node.0].push(edge);

        Ok(())
    }

    pub fn disconnect(&mut self, src: PortRef, dst: PortRef) -> Result<(), Error> {
        self.port_check(src)?;
        self.port_check(dst)?;
        let (src_node, _) = self.port_data[src.0];
        let (dst_node, _) = self.port_data[dst.0];
        let type_ = self.port_data[src.0].1;
        self.remove_edge(Edge {
            src_node,
            src_port: src,
            dst_node,
            dst_port: dst,
            type_,
        })
    }

    pub fn set_delay(&mut self, node: NodeRef, delay: u64) -> Result<(), Error> {
        self.node_check(node)?;
        self.delays[node.0] = delay;
        Ok(())
    }

    pub fn port_ident(&self, port: PortRef) -> Result<&'_ P, Error> {
        self.port_check(port)?;
        Ok(&self.port_identifiers[port.0])
    }

    pub fn node_ident(&self, node: NodeRef) -> Result<&'_ N, Error> {
        self.node_check(node)?;
        Ok(&self.node_identifiers[node.0])
    }

    pub fn set_port_ident(&mut self, port: PortRef, ident: P) -> Result<(), Error> {
        self.port_check(port)?;
        self.port_identifiers[port.0] = ident;
        Ok(())
    }

    pub fn set_node_ident(&mut self, node: NodeRef, ident: N) -> Result<(), Error> {
        self.node_check(node)?;
        self.node_identifiers[node.0] = ident;
        Ok(())
    }

    pub fn node_check(&self, node: NodeRef) -> Result<(), Error> {
        if node.0 < self.node_count() && !self.free_nodes.contains(&node) {
            Ok(())
        } else {
            Err(Error::NodeDoesNotExist)
        }
    }

    pub fn port_check(&self, port: PortRef) -> Result<(), Error> {
        if port.0 < self.port_count() && !self.free_ports.contains(&port) {
            Ok(())
        } else {
            Err(Error::PortDoesNotExist)
        }
    }

    fn node_count(&self) -> usize {
        self.ports.len()
    }

    fn port_count(&self) -> usize {
        self.port_data.len()
    }

    /// Check that adding an edge `src` -> `dst` won't create a cycle. Must be called
    /// before each edge addition.
    ///
    /// TODO: Optimize for adding multiple edges at once. (pass over the whole graph)
    fn cycle_check(&mut self, src: NodeRef, dst: NodeRef) -> Result<(), Error> {
        // This won't panic because this is always `Some` on the user's end.
        let mut heap_store = self.heap_store.take().unwrap();
        let mut queue = heap_store.walk_queue.take().unwrap();
        let mut queued = heap_store.cycle_queued.take().unwrap();

        queue.clear();
        queued.clear();
        queue.push_back(dst);
        queued.insert(dst);

        while let Some(node) = queue.pop_front() {
            if node == src {
                heap_store.walk_queue = Some(queue);
                heap_store.cycle_queued = Some(queued);
                self.heap_store = Some(heap_store);

                return Err(Error::Cycle);
            }
            for dependent in self.dependents(node) {
                if !queued.contains(&dependent) {
                    queue.push_back(dependent);
                    queued.insert(dependent);
                }
            }
        }

        heap_store.walk_queue = Some(queue);
        heap_store.cycle_queued = Some(queued);
        self.heap_store = Some(heap_store);

        Ok(())
    }

    fn remove_edge(&mut self, edge: Edge<PT>) -> Result<(), Error> {
        let Edge {
            src_node, dst_node, ..
        } = edge;
        let src_index = self.edges[src_node.0].iter().position(|e| *e == edge);
        let dst_index = self.edges[dst_node.0].iter().position(|e| *e == edge);
        match (src_index, dst_index) {
            (Some(s), Some(d)) => {
                self.edges[src_node.0].remove(s);
                self.edges[dst_node.0].remove(d);

                Ok(())
            }
            _ => Err(Error::ConnectionDoesNotExist),
        }
    }

    fn incoming(&self, port: PortRef) -> impl Iterator<Item = Edge<PT>> + '_ {
        let node = self.port_data[port.0].0;
        self.edges[node.0]
            .iter()
            .filter(move |e| e.dst_port == port)
            .copied()
    }

    fn outgoing(&self, port: PortRef) -> impl Iterator<Item = Edge<PT>> + '_ {
        let node = self.port_data[port.0].0;

        self.edges[node.0]
            .iter()
            .filter(move |e| e.src_port == port)
            .copied()
    }

    fn dependencies(&self, node: NodeRef) -> impl Iterator<Item = NodeRef> + '_ {
        self.edges[node.0].iter().filter_map(move |e| {
            if e.dst_node == node {
                Some(e.src_node)
            } else {
                None
            }
        })
    }

    fn dependents(&self, node: NodeRef) -> impl Iterator<Item = NodeRef> + '_ {
        self.edges[node.0].iter().filter_map(move |e| {
            if e.src_node == node {
                Some(e.dst_node)
            } else {
                None
            }
        })
    }

    /// Walk graph in topological order using Kahn's algorithm.
    fn walk_mut(
        &mut self,
        heap_store: &mut HeapStore<N, P, PT>,
        queue: &mut VecDeque<NodeRef>,
        indegree: &mut FnvHashMap<NodeRef, usize>,
        mut f: impl FnMut(&mut Graph<N, P, PT>, NodeRef, &mut HeapStore<N, P, PT>),
    ) {
        queue.clear();
        indegree.clear();

        for node_index in 0..self.node_count() {
            indegree.insert(NodeRef(node_index), 0);
        }
        for node in &self.free_nodes {
            indegree.remove(node);
        }

        for (&node, value) in indegree.iter_mut() {
            *value = self.dependencies(node).count();
            if *value == 0 {
                queue.push_back(node);
            }
        }

        while let Some(node) = queue.pop_front() {
            (&mut f)(self, node, heap_store);
            for dependent in self.dependents(node) {
                let value = indegree
                    .get_mut(&dependent)
                    .expect("edge refers to freed node");
                *value = value.checked_sub(1).expect("corrupted graph");
                if *value == 0 {
                    queue.push_back(dependent);
                }
            }
        }
    }

    pub fn compile(&mut self) -> &[Scheduled<N, P, PT>] {
        let solve_latency_requirements =
            |graph: &mut Graph<N, P, PT>,
             node: NodeRef,
             heap_store: &mut HeapStore<N, P, PT>,
             delay_comps: &mut FnvHashMap<(PortRef, PortRef), u64>| {
                heap_store.deps.clear();
                heap_store.latencies.clear();
                for edge in graph.edges[node.0].iter().filter(|e| e.dst_node == node) {
                    heap_store.deps.push(edge.src_node);

                    heap_store.latencies.push(
                        heap_store.all_latencies[edge.src_node.0].unwrap()
                            + graph.delays[edge.src_node.0],
                    );
                }

                let max_latency = heap_store.latencies.iter().max().copied().or(Some(0));

                heap_store.all_latencies[node.0] = max_latency;

                for (dep, latency) in heap_store.deps.iter().zip(heap_store.latencies.iter()) {
                    let compensation = max_latency.unwrap() - latency;
                    if compensation != 0 {
                        for edge in graph.edges[node.0].iter().filter(|e| e.src_node == *dep) {
                            let _ =
                                delay_comps.insert((edge.src_port, edge.dst_port), compensation);
                        }
                    }
                }
            };

        let solve_buffer_requirements =
            |graph: &Graph<N, P, PT>, node: NodeRef, heap_store: &mut HeapStore<N, P, PT>| {
                for port in &graph.ports[node.0] {
                    let (_, type_) = graph.port_data[port.0];

                    for output in graph.outgoing(*port) {
                        let (buffer, count) = heap_store
                            .output_assignments
                            .entry((node, *port))
                            .or_insert((heap_store.allocator.acquire(type_), 0));
                        *count += 1;
                        heap_store
                            .input_assignments
                            .entry((output.dst_node, output.dst_port))
                            .or_insert(vec![])
                            .push((*buffer, (output.src_port, output.dst_port)));
                    }
                    for input in graph.incoming(*port) {
                        let (buffer, count) = heap_store
                            .output_assignments
                            .get_mut(&(input.src_node, input.src_port))
                            .expect("no output buffer assigned");
                        *count -= 1;
                        if *count == 0 {
                            heap_store.allocator.release(*buffer);
                        }
                    }
                }
            };

        // This won't panic because this is always `Some` on the user's end.
        let mut heap_store = self.heap_store.take().unwrap();

        let mut scheduled = heap_store.scheduled.take().unwrap_or_default();
        scheduled.clear();

        heap_store.all_latencies.clear();
        heap_store.all_latencies.resize(self.node_count(), None);

        let mut delay_comps = heap_store.delay_comps.take().unwrap();
        delay_comps.clear();

        heap_store.allocator.clear();
        heap_store.input_assignments.clear();
        heap_store.output_assignments.clear();

        let mut scheduled_nodes = heap_store.scheduled_nodes.take().unwrap();
        scheduled_nodes.clear();

        let mut queue = heap_store.walk_queue.take().unwrap();
        let mut indegree = heap_store.walk_indegree.take().unwrap();

        self.walk_mut(
            &mut heap_store,
            &mut queue,
            &mut indegree,
            |graph, node, heap_store| {
                // TODO: Maybe use the log crate for this to avoid polluting the user's output?
                // println!("compiling {}", graph.node_name(node).unwrap());

                solve_latency_requirements(graph, node, heap_store, &mut delay_comps);
                solve_buffer_requirements(graph, node, heap_store);
                scheduled_nodes.push(node);
            },
        );

        for node in scheduled_nodes.iter() {
            let node_ident = self.node_identifiers[node.0].clone();

            let inputs = self.ports[node.0]
                .iter()
                .filter_map(|port| {
                    heap_store
                        .input_assignments
                        .get(&(*node, *port))
                        .map(|buffers| {
                            let buffers = buffers
                                .iter()
                                .map(|(buffer, ports)| {
                                    let delay_comp = delay_comps.get(ports).copied().unwrap_or(0);

                                    (*buffer, delay_comp)
                                })
                                .collect();

                            (self.port_identifiers[port.0].clone(), buffers)
                        })
                })
                .collect::<Vec<_>>();
            let outputs = self.ports[node.0]
                .iter()
                .filter_map(|port| {
                    heap_store
                        .output_assignments
                        .get(&(*node, *port))
                        .map(|(buffer, _)| (self.port_identifiers[port.0].clone(), *buffer))
                })
                .collect::<Vec<_>>();

            scheduled.push(Scheduled {
                node: node_ident,
                inputs,
                outputs,
            });
        }

        heap_store.scheduled = Some(scheduled);
        heap_store.delay_comps = Some(delay_comps);
        heap_store.scheduled_nodes = Some(scheduled_nodes);
        heap_store.walk_queue = Some(queue);
        heap_store.walk_indegree = Some(indegree);

        self.heap_store = Some(heap_store);

        self.heap_store
            .as_ref()
            .unwrap()
            .scheduled
            .as_ref()
            .unwrap()
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Error {
    NodeDoesNotExist,
    PortDoesNotExist,
    Cycle,
    ConnectionDoesNotExist,
    RefDoesNotExist,
    InvalidPortType,
}

impl std::error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NodeDoesNotExist => write!(f, "Audio graph node does not exist"),
            Error::PortDoesNotExist => write!(f, "Audio graph port does not exist"),
            Error::Cycle => write!(f, "Audio graph cycle detected"),
            Error::ConnectionDoesNotExist => write!(f, "Audio graph connection does not exist"),
            Error::RefDoesNotExist => write!(f, "Audio graph reference does not exist"),
            Error::InvalidPortType => write!(
                f,
                "Cannot connect audio graph ports. Ports are a different type"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_ops() {
        let mut graph = Graph::default();
        let a = graph.node("A");
        let b = graph.node("B");

        let a_in = graph
            .port(a, DefaultPortType::Event, "events")
            .expect("port was not created");
        let a_out = graph
            .port(a, DefaultPortType::Audio, "output")
            .expect("port was not created");
        let b_in = graph
            .port(b, DefaultPortType::Audio, "input")
            .expect("port was not created");

        dbg!(&graph.port_count());
        graph.connect(a_out, b_in).expect("could not connect");
        graph
            .connect(a_in, b_in)
            .expect_err("connected mistyped ports");
        graph.delete_port(a_in).expect("could not delete port");
        graph
            .disconnect(a_out, b_in)
            .expect("could not disconnect ports");
        graph.delete_node(a).expect("could not delete");
        graph
            .connect(a_out, b_in)
            .expect_err("connected node that doesn't exist");
    }

    #[test]
    fn simple_graph() {
        let mut graph = Graph::default();
        let (a, b, c, d) = (
            graph.node("A"),
            graph.node("B"),
            graph.node("C"),
            graph.node("D"),
        );
        let (a_out, b_out, c_out) = (
            graph
                .port(a, DefaultPortType::Audio, "output")
                .expect("could not create output port"),
            graph
                .port(b, DefaultPortType::Audio, "output")
                .expect("could not create output port"),
            graph
                .port(c, DefaultPortType::Audio, "output")
                .expect("could not create output port"),
        );

        let (a_in, b_in, c_in, d_in, d_in_2) = (
            graph
                .port(a, DefaultPortType::Audio, "input")
                .expect("could not create input"),
            graph
                .port(b, DefaultPortType::Audio, "input")
                .expect("could not create input"),
            graph
                .port(c, DefaultPortType::Audio, "input")
                .expect("could not create input"),
            graph
                .port(d, DefaultPortType::Audio, "d_input_1")
                .expect("could not create input"),
            graph
                .port(d, DefaultPortType::Audio, "d_input_2")
                .expect("could not create input"),
        );
        graph.set_delay(b, 2).expect("could not update delay of b");
        graph.set_delay(c, 5).expect("could not update delay of c");
        graph.connect(a_out, b_in).expect("could not connect");
        graph.connect(a_out, c_in).expect("could not connect");
        graph.connect(b_out, d_in).expect("could not connect");
        graph.connect(c_out, d_in).expect("could not connect");
        graph.connect(b_out, d_in_2).expect("could not connect");

        graph
            .connect(b_out, a_in)
            .expect_err("Cycles should not be allowed");

        let mut last_node = None;
        for entry in graph.compile() {
            println!("process {:?}:", entry.node);
            for (port, buffers) in entry.inputs.iter() {
                println!("    {} => ", port);

                if *port == "d_input_1" {
                    for (b, delay_comp) in buffers {
                        println!("        index: {}", b.index);
                        println!("        delay_comp: {}", delay_comp);
                    }

                    // One of the buffers should have a delay_comp of 0, and one
                    // should have a delay_comp of 3
                    assert!(
                        (buffers[0].1 == 0 && buffers[1].1 == 3)
                            || (buffers[0].1 == 3 && buffers[1].1 == 0)
                    )
                } else {
                    for (b, delay_comp) in buffers {
                        println!("        index: {}", b.index);
                        println!("        delay_comp: {}", delay_comp);

                        if *port == "d_input_2" {
                            assert_eq!(*delay_comp, 3);
                        } else {
                            assert_eq!(*delay_comp, 0);
                        }
                    }
                }
            }
            for (port, buffer) in entry.outputs.iter() {
                println!("    {:?} => {}", port, buffer.index);
            }
            last_node = Some(entry.node.clone());
        }
        assert!(matches!(last_node, Some("D")));
    }
}
