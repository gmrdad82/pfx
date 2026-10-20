@group(0) @binding(21) var<storage, read_write> trace_counters: array<atomic<u32>>;
var<private> trace_counts: array<u32,10>;
fn trace_count(kind: u32) {
    trace_counts[kind]+=1u;
}
fn trace_count_by(kind: u32, n: u32) {
    trace_counts[kind]+=n;
}
fn trace_count_add(kind: u32, n: u32) {
    let low=atomicAdd(&trace_counters[2u*kind],n);
    if (low>0xffffffffu-n) { atomicAdd(&trace_counters[2u*kind+1u],1u); }
}
fn trace_count_flush() {
    for (var k=0u;k<COUNT_KINDS;k++) {
        let total=trace_count_reduce(trace_counts[k]);
        let first=trace_count_first();
        if (first && total>0u) { trace_count_add(k,total); }
    }
}
