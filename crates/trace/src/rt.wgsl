@group(0) @binding(17) var rt_world: acceleration_structure;
@group(0) @binding(18) var<storage, read> rt_map: array<u32>;
fn rt_triangle(c: RayIntersection) -> u32 {
    if (c.instance_custom_data>0u) { return rt_map[rt_map[2]+c.instance_custom_data-1u]+c.primitive_index; }
    return rt_map[3u+select(0u,rt_map[0],c.geometry_index==1u)+c.primitive_index];
}
fn rt_record(c: RayIntersection) -> u32 {
    return c.instance_custom_data;
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
fn rt_shape_hit(i: u32, o: vec3f, d: vec3f, t: f32) -> Hit {
    let p=o+d*t;
    let e=0.0002;
    let n=normalize(vec3f(
        shape_distance(i,p+vec3f(e,0.0,0.0))-shape_distance(i,p-vec3f(e,0.0,0.0)),
        shape_distance(i,p+vec3f(0.0,e,0.0))-shape_distance(i,p-vec3f(0.0,e,0.0)),
        shape_distance(i,p+vec3f(0.0,0.0,e))-shape_distance(i,p-vec3f(0.0,0.0,e))));
    return Hit(t,n,n,u32(shapes[i].c.w),vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,i+1u,0u);
}
const RT_MOVERS: u32 = 16777215u;
fn node_at(i: u32) -> Node {
    let at=frame.columns.z+(i-frame.columns.w)*12u;
    return Node(bitcast<vec4f>(vec4u(rt_map[at],rt_map[at+1u],rt_map[at+2u],rt_map[at+3u])),
        bitcast<vec4f>(vec4u(rt_map[at+4u],rt_map[at+5u],rt_map[at+6u],rt_map[at+7u])),
        rt_map[at+8u],rt_map[at+9u],vec2u(rt_map[at+10u],rt_map[at+11u]));
}
fn motion_vec(k: u32) -> vec4f {
    let at=frame.columns.y+k*4u;
    return bitcast<vec4f>(vec4u(rt_map[at],rt_map[at+1u],rt_map[at+2u],rt_map[at+3u]));
}
fn empty_hit(limit: f32) -> Hit {
    return Hit(limit,vec3f(0.0),vec3f(0.0),0u,vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,0u,0u);
}
fn mover_walk(s: u32, o: vec3f, d: vec3f, limit: f32, mode: u32, shadow_ray: bool, hit: ptr<function,Hit>) -> vec3f {
    let m=mover_enter(s,o,d);
    let at=slot_at(s);
    let base=motion_word(at).w;
    let record=s+1u;
    let word=instance_surfaces[record].info.x;
    var stack: array<u32,64>;
    var top=1u;
    stack[0]=motion_word(at+1u).x;
    var passed=vec3f(1.0);
    while (top>0u) {
        top-=1u;
        let index=stack[top];
        let node=node_at(index);
        var far=limit;
        if (mode==0u) { far=(*hit).t; }
        if (!box_hit(node,m.ro,m.rd,far)) { continue; }
        if (node.count==0u) {
            if (top+2u<=64u) {
                stack[top]=node.first;
                stack[top+1u]=index+1u;
                top+=2u;
            }
        } else {
            for (var j=0u;j<node.count;j++) {
                let i=base+node.first+j;
                if (mode==0u) {
                    tri_hit(i,record,word,m.inv_det,m.ro,m.rd,o,d,shadow_ray,hit);
                } else if (mode==1u) {
                    if (tri_blocks(i,record,word,m.inv_det,m.ro,m.rd,o,d,limit)) {
                        mover_leave();
                        return vec3f(0.0);
                    }
                } else {
                    passed*=tri_passes(i,record,word,m.inv_det,m.ro,m.rd,o,d,limit);
                }
            }
            if (all(passed<=vec3f(0.0))) { break; }
        }
    }
    mover_leave();
    return passed;
}
fn scene(o: vec3f, d: vec3f, limit: f32, shadow_ray: bool) -> Hit {
    var hit=Hit(limit,vec3f(0.0),vec3f(0.0),0u,vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,0u,0u);
    if (!rt_valid(o,d,limit)) { return hit; }
    var rq: ray_query;
    rayQueryInitialize(&rq,rt_world,RayDesc(RAY_FLAG_NONE,0xffu,0.0001,limit,o,d));
    var best=limit;
    var shape=0u;
    var mover=0u;
    var moved=hit;
    var pending=array<u32,4>(0u,0u,0u,0u);
    var waiting=0u;
    while (rayQueryProceed(&rq)) {
        let c=rayQueryGetCandidateIntersection(&rq);
        if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
            var probe=hit;
            probe.t=best;
            let record=rt_record(c);
            let info=instance_surfaces[record].info;
            tri_hit(rt_triangle(c),record,info.x,bitcast<f32>(info.w),record_ray(record,o,1.0),record_ray(record,d,0.0),o,d,shadow_ray,&probe);
            if (probe.t<best) {
                rayQueryConfirmIntersection(&rq);
                best=probe.t;
                shape=0u;
                mover=0u;
            }
        } else if (TRACE_MOTION && c.instance_custom_data==RT_MOVERS) {
            let slot=mover_slot(c.primitive_index);
            if (waiting<4u) {
                pending[waiting]=slot;
                waiting+=1u;
            } else {
                var probe=empty_hit(best);
                _ = mover_walk(slot,o,d,best,0u,shadow_ray,&probe);
                if (probe.t<best) {
                    best=probe.t;
                    moved=probe;
                    mover=slot+1u;
                    shape=0u;
                }
            }
        } else {
            let i=rt_shape(c);
            let t=rt_shape_t(i,o,d,best);
            if (t<best && t>0.0001) {
                best=t;
                shape=i+1u;
                mover=0u;
            }
        }
    }
    let committed=rayQueryGetCommittedIntersection(&rq);
    let triangle=committed.kind==RAY_QUERY_INTERSECTION_TRIANGLE;
    if (TRACE_MOTION) {
        var reach=best;
        if (triangle) { reach=min(reach,committed.t); }
        for (var k=0u;k<waiting;k++) {
            var probe=empty_hit(reach);
            _ = mover_walk(pending[k],o,d,reach,0u,shadow_ray,&probe);
            if (probe.t<reach) {
                reach=probe.t;
                best=probe.t;
                moved=probe;
                mover=pending[k]+1u;
                shape=0u;
            }
        }
    }
    if (shape>0u && !(triangle && committed.t<best)) {
        hit=rt_shape_hit(shape-1u,o,d,best);
    } else if (TRACE_MOTION && mover>0u && !(triangle && committed.t<best)) {
        hit=moved;
    } else if (triangle) {
        tri_fill(rt_triangle(committed),rt_record(committed),committed.t,committed.barycentrics.x,committed.barycentrics.y,&hit);
    }
    if (!RT_BOXES) {
        for (var i=0u;i<frame.counts.y;i++) {
            if (shapes[i].d.x > 0.5) { continue; }
            let t=rt_shape_t(i,o,d,hit.t);
            if (t<hit.t && t>0.0001) { hit=rt_shape_hit(i,o,d,t); }
        }
    }
    return world_hit(hit);
}
fn occluded(o: vec3f, d: vec3f, limit: f32) -> bool {
    if (!rt_valid(o,d,limit)) { return true; }
    var rq: ray_query;
    rayQueryInitialize(&rq,rt_world,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,0xffu,0.0001,limit,o,d));
    var blocked=false;
    var pending=array<u32,4>(0u,0u,0u,0u);
    var waiting=0u;
    while (rayQueryProceed(&rq)) {
        let c=rayQueryGetCandidateIntersection(&rq);
        if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
            let record=rt_record(c);
            let info=instance_surfaces[record].info;
            if (tri_blocks(rt_triangle(c),record,info.x,bitcast<f32>(info.w),record_ray(record,o,1.0),record_ray(record,d,0.0),o,d,limit)) { rayQueryConfirmIntersection(&rq); }
        } else if (TRACE_MOTION && c.instance_custom_data==RT_MOVERS) {
            let slot=mover_slot(c.primitive_index);
            if (waiting<4u) {
                pending[waiting]=slot;
                waiting+=1u;
            } else {
                var none=empty_hit(limit);
                if (all(mover_walk(slot,o,d,limit,1u,true,&none)<=vec3f(0.0))) {
                    blocked=true;
                    rayQueryTerminate(&rq);
                }
            }
        } else {
            let t=rt_shape_t(rt_shape(c),o,d,limit);
            if (t<limit && t>0.0001) {
                blocked=true;
                rayQueryTerminate(&rq);
            }
        }
    }
    if (blocked || rayQueryGetCommittedIntersection(&rq).kind!=RAY_QUERY_INTERSECTION_NONE) { return true; }
    if (TRACE_MOTION) {
        for (var k=0u;k<waiting;k++) {
            var none=empty_hit(limit);
            if (all(mover_walk(pending[k],o,d,limit,1u,true,&none)<=vec3f(0.0))) { return true; }
        }
    }
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
    var pending=array<u32,4>(0u,0u,0u,0u);
    var waiting=0u;
    rayQueryInitialize(&rq,rt_world,RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT,0xffu,0.0001,limit,o,d));
    while (rayQueryProceed(&rq)) {
        let c=rayQueryGetCandidateIntersection(&rq);
        if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
            let record=rt_record(c);
            let info=instance_surfaces[record].info;
            passed*=tri_passes(rt_triangle(c),record,info.x,bitcast<f32>(info.w),record_ray(record,o,1.0),record_ray(record,d,0.0),o,d,limit);
        } else if (TRACE_MOTION && c.instance_custom_data==RT_MOVERS) {
            let slot=mover_slot(c.primitive_index);
            if (waiting<4u) {
                pending[waiting]=slot;
                waiting+=1u;
            } else {
                var none=empty_hit(limit);
                passed*=mover_walk(slot,o,d,limit,2u,true,&none);
            }
        } else {
            let i=rt_shape(c);
            let t=rt_shape_t(i,o,d,limit);
            if (t<limit && t>0.0001) { passed*=cast_tint(u32(shapes[i].c.w)); }
        }
        if (all(passed<=vec3f(0.0))) { rayQueryTerminate(&rq); }
    }
    if (rayQueryGetCommittedIntersection(&rq).kind!=RAY_QUERY_INTERSECTION_NONE) { return vec3f(0.0); }
    if (TRACE_MOTION) {
        for (var k=0u;k<waiting;k++) {
            var none=empty_hit(limit);
            passed*=mover_walk(pending[k],o,d,limit,2u,true,&none);
            if (all(passed<=vec3f(0.0))) { return vec3f(0.0); }
        }
    }
    if (!RT_BOXES) {
        for (var i=0u;i<frame.counts.y;i++) {
            if (shapes[i].d.x > 0.5) { continue; }
            let t=rt_shape_t(i,o,d,limit);
            if (t<limit && t>0.0001) { passed*=cast_tint(u32(shapes[i].c.w)); }
        }
    }
    return passed;
}
