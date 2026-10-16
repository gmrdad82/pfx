@group(0) @binding(17) var rt_world: acceleration_structure;
@group(0) @binding(18) var<storage, read> rt_map: array<u32>;
fn rt_triangle(c: RayIntersection) -> u32 {
    if (c.instance_custom_data>0u) { return rt_map[rt_map[2]+c.instance_custom_data-1u]+c.primitive_index; }
    return rt_map[3u+select(0u,rt_map[0],c.geometry_index==1u)+c.primitive_index];
}
fn rt_shape(c: RayIntersection) -> u32 {
    return rt_map[3u+rt_map[1]+c.primitive_index];
}
fn rt_finite(v: f32) -> bool {
    return (bitcast<u32>(v)&0x7fffffffu)<0x7f800000u;
}
fn rt_valid(o: vec3f, d: vec3f, limit: f32) -> bool {
    return rt_finite(o.x) && rt_finite(o.y) && rt_finite(o.z) && rt_finite(d.x) && rt_finite(d.y) && rt_finite(d.z) && (bitcast<u32>(limit)&0x7fffffffu)<=0x7f800000u;
}
fn rt_shape_t(i: u32, o: vec3f, d: vec3f, limit: f32) -> f32 {
    if (u32(shapes[i].a.w)==5u) { return union_hit(i,o,d,limit); }
    return primitive_hit(i,o,d,limit);
}
fn rt_fill(i: u32, o: vec3f, d: vec3f, t: f32, u: f32, v: f32, hit: ptr<function,Hit>) {
    let tri = triangles[i];
    let e1 = tri.b.xyz-tri.a.xyz;
    let e2 = tri.c.xyz-tri.a.xyz;
    (*hit).t=t;
    (*hit).surface=i;
    (*hit).shape=0u;
    let geometric=normalize(cross(e1,e2));
    (*hit).geometric=geometric;
    let shaded=tri.na.xyz*(1.0-u-v)+tri.nb.xyz*u+tri.nc.xyz*v;
    (*hit).n=select(geometric,normalize(shaded),dot(shaded,shaded)>1e-12);
    (*hit).material=u32(tri.a.w);
    let uv_v=triangle_uv_v[i];
    (*hit).uv=vec2f(tri.na.w*(1.0-u-v)+tri.nb.w*u+tri.nc.w*v,
        uv_v.x*(1.0-u-v)+uv_v.y*u+uv_v.z*v);
    let du1=tri.nb.w-tri.na.w;
    let du2=tri.nc.w-tri.na.w;
    let dv1=uv_v.y-uv_v.x;
    let dv2=uv_v.z-uv_v.x;
    let uv_det=du1*dv2-du2*dv1;
    var tangent=normalize(e1);
    if (abs(uv_det)>1e-10) { tangent=normalize((e1*dv2-e2*dv1)/uv_det); }
    (*hit).tangent=tangent;
    (*hit).bitangent=normalize(cross(geometric,tangent))*select(-1.0,1.0,uv_det>=0.0);
}
fn rt_shape_hit(i: u32, o: vec3f, d: vec3f, t: f32) -> Hit {
    let p=o+d*t;
    let e=0.0002;
    let n=normalize(vec3f(
        shape_distance(i,p+vec3f(e,0.0,0.0))-shape_distance(i,p-vec3f(e,0.0,0.0)),
        shape_distance(i,p+vec3f(0.0,e,0.0))-shape_distance(i,p-vec3f(0.0,e,0.0)),
        shape_distance(i,p+vec3f(0.0,0.0,e))-shape_distance(i,p-vec3f(0.0,0.0,e))));
    return Hit(t,n,n,u32(shapes[i].c.w),vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,i+1u);
}
fn scene(o: vec3f, d: vec3f, limit: f32, shadow_ray: bool) -> Hit {
    var hit=Hit(limit,vec3f(0.0),vec3f(0.0),0u,vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,0u);
    if (!rt_valid(o,d,limit)) { return hit; }
    var rq: ray_query;
    rayQueryInitialize(&rq,rt_world,RayDesc(RAY_FLAG_NONE,0xffu,0.0001,limit,o,d));
    var best=limit;
    var shape=0u;
    while (rayQueryProceed(&rq)) {
        let c=rayQueryGetCandidateIntersection(&rq);
        if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
            var probe=hit;
            probe.t=best;
            tri_hit(rt_triangle(c),o,d,shadow_ray,&probe);
            if (probe.t<best) {
                rayQueryConfirmIntersection(&rq);
                best=probe.t;
                shape=0u;
            }
        } else {
            let i=rt_shape(c);
            let t=rt_shape_t(i,o,d,best);
            if (t<best && t>0.0001) {
                best=t;
                shape=i+1u;
            }
        }
    }
    let committed=rayQueryGetCommittedIntersection(&rq);
    let triangle=committed.kind==RAY_QUERY_INTERSECTION_TRIANGLE;
    if (shape>0u && !(triangle && committed.t<best)) {
        hit=rt_shape_hit(shape-1u,o,d,best);
    } else if (triangle) {
        rt_fill(rt_triangle(committed),o,d,committed.t,committed.barycentrics.x,committed.barycentrics.y,&hit);
    }
    if (!RT_BOXES) {
        for (var i=0u;i<frame.counts.y;i++) {
            if (shapes[i].d.x > 0.5) { continue; }
            let t=rt_shape_t(i,o,d,hit.t);
            if (t<hit.t && t>0.0001) { hit=rt_shape_hit(i,o,d,t); }
        }
    }
    return hit;
}
fn occluded(o: vec3f, d: vec3f, limit: f32) -> bool {
    if (!rt_valid(o,d,limit)) { return true; }
    var rq: ray_query;
    rayQueryInitialize(&rq,rt_world,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,0xffu,0.0001,limit,o,d));
    var blocked=false;
    while (rayQueryProceed(&rq)) {
        let c=rayQueryGetCandidateIntersection(&rq);
        if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
            if (tri_blocks(rt_triangle(c),o,d,limit)) { rayQueryConfirmIntersection(&rq); }
        } else {
            let t=rt_shape_t(rt_shape(c),o,d,limit);
            if (t<limit && t>0.0001) {
                blocked=true;
                rayQueryTerminate(&rq);
            }
        }
    }
    if (blocked || rayQueryGetCommittedIntersection(&rq).kind!=RAY_QUERY_INTERSECTION_NONE) { return true; }
    if (!RT_BOXES) {
        for (var i=0u;i<frame.counts.y;i++) {
            if (shapes[i].d.x > 0.5) { continue; }
            let t=rt_shape_t(i,o,d,limit);
            if (t<limit && t>0.0001) { return true; }
        }
    }
    return false;
}
fn shadow_passed(o: vec3f, d: vec3f, limit: f32) -> vec3f {
    if (!rt_valid(o,d,limit)) { return vec3f(0.0); }
    var passed=vec3f(1.0);
    var rq: ray_query;
    rayQueryInitialize(&rq,rt_world,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,0xffu,0.0001,limit,o,d));
    while (rayQueryProceed(&rq)) {
        let c=rayQueryGetCandidateIntersection(&rq);
        if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
            passed*=tri_passes(rt_triangle(c),o,d,limit);
        } else {
            let i=rt_shape(c);
            let t=rt_shape_t(i,o,d,limit);
            if (t<limit && t>0.0001) { passed*=cast_tint(u32(shapes[i].c.w)); }
        }
        if (all(passed<=vec3f(0.0))) { rayQueryTerminate(&rq); }
    }
    if (rayQueryGetCommittedIntersection(&rq).kind!=RAY_QUERY_INTERSECTION_NONE) { return vec3f(0.0); }
    if (!RT_BOXES) {
        for (var i=0u;i<frame.counts.y;i++) {
            if (shapes[i].d.x > 0.5) { continue; }
            let t=rt_shape_t(i,o,d,limit);
            if (t<limit && t>0.0001) { passed*=cast_tint(u32(shapes[i].c.w)); }
        }
    }
    return passed;
}
