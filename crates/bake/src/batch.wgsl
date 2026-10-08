struct FaceRecord {
    origin: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
    seed: u32,
    pad: array<u32, 3>,
}
@group(0) @binding(20) var<storage, read> faces: array<FaceRecord>;

@compute @workgroup_size(8,8)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    if (gid.x>=frame.size.x || gid.y>=frame.size.y) { return; }
    let face_index=(gid.y/4u)*(frame.size.x/4u)+gid.x/4u;
    if (face_index>=frame.counts.w) { return; }
    let face=faces[face_index];
    let local=gid.xy%vec2u(4u);
    let local_index=local.y*4u+local.x;
    let index=gid.y*frame.size.x+gid.x;
    var sum=Accum(vec4f(0.0),vec4f(0.0),vec4f(0.0));
    if (frame.size.z>0u) { sum=accum[index]; }
    for (var sample=0u;sample<frame.size.w;sample++) {
        var first_albedo=vec4f(0.0);
        var first_normal=vec4f(0.0);
        var state=pcg(local_index*1973u+pcg(face.seed*9277u+(frame.size.z+sample)*7919u+26699u));
        let jitter=vec2f(random(&state),random(&state));
        let ndc=(vec2f(local)+jitter)/vec2f(4.0)*2.0-1.0;
        let direction=normalize(face.forward.xyz+face.right.xyz*ndc.x+face.up.xyz*(-ndc.y));
        let value=radiance(face.origin.xyz,direction,&state,&first_albedo,&first_normal);
        sum.color=vec4f(sum.color.xyz+value,sum.color.w);
        sum.albedo+=first_albedo;
        sum.normal+=first_normal;
    }
    sum.color.w=f32(frame.size.z+frame.size.w);
    accum[index]=sum;
    let count=max(sum.color.w,1.0);
    let n=sum.normal.xyz/count;
    textureStore(output,vec2i(gid.xy),vec4f(sum.color.xyz/count,1.0));
    textureStore(albedo,vec2i(gid.xy),sum.albedo/count);
    textureStore(normal,vec2i(gid.xy),vec4f(select(vec3f(0.0),normalize(n),length(n)>1e-6),sum.normal.w/count));
}
