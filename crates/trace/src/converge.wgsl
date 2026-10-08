struct Accum { color: vec4f, albedo: vec4f, normal: vec4f }
struct Snap { mean: vec4f, info: vec4f }
struct Check { size: vec4u, limits: vec4f }
@group(0) @binding(0) var<uniform> check: Check;
@group(0) @binding(1) var<storage, read_write> accum: array<Accum>;
@group(0) @binding(2) var<storage, read_write> snaps: array<Snap>;
@group(0) @binding(3) var<storage, read_write> running: array<atomic<u32>>;

@compute @workgroup_size(8,8)
fn measure(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x>=check.size.x || gid.y>=check.size.y) { return; }
    let index=gid.y*check.size.x+gid.x;
    let sum=accum[index];
    let count=sum.color.w;
    if (count<=0.0) {
        snaps[index].info.y=0.0;
        return;
    }
    let now=vec4f(sum.color.xyz/count,sum.albedo.w/count);
    let snap=snaps[index];
    let prior=snap.info.x;
    var error=1e9;
    if (prior>0.0 && count>prior) {
        let rest=(now*count-snap.mean*prior)/(count-prior);
        let scale=sqrt(1.0/count)/sqrt(1.0/prior+1.0/(count-prior));
        let gap=abs(snap.mean-rest)*scale;
        error=(gap.x+gap.y+gap.z)/(1e-4+sqrt(max(now.x+now.y+now.z,0.0)))+gap.w;
    }
    snaps[index]=Snap(now,vec4f(count,error,0.0,0.0));
}

@compute @workgroup_size(8,8)
fn mark(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x>=check.size.x || gid.y>=check.size.y) { return; }
    let index=gid.y*check.size.x+gid.x;
    let count=accum[index].color.w;
    if (count<=0.0) { return; }
    var worst=0.0;
    for (var dy=-1;dy<=1;dy++) {
        for (var dx=-1;dx<=1;dx++) {
            let x=clamp(i32(gid.x)+dx,0,i32(check.size.x)-1);
            let y=clamp(i32(gid.y)+dy,0,i32(check.size.y)-1);
            worst=max(worst,snaps[u32(y)*check.size.x+u32(x)].info.y);
        }
    }
    if (count>=f32(check.size.z) && worst<check.limits.x) {
        accum[index].color.w=-count;
    } else {
        atomicAdd(&running[0],1u);
    }
}
