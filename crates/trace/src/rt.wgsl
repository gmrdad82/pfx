@group(0) @binding(17) var rt_world: acceleration_structure;
@group(0) @binding(18) var<storage, read> rt_map: array<u32>;
@group(0) @binding(20) var rt_movers: acceleration_structure;
fn rt_mask() -> u32 {
    return select(0xffu,0xfeu,TRACE_SPRITES && sprites_hidden);
}
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
const RT_MOVERS: u32 = 16777215u;
const RT_MOVER_MASK: u32 = 2u;
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
fn mover_walk(s: u32, o: vec3f, d: vec3f, limit: f32, mode: u32, shadow_ray: bool, hit: ptr<function,Hit>) -> vec3f {
    trace_count(COUNT_MOVER_WALKS);
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
struct MoverRay { m: Moved, fo: vec3f, fd: vec3f, word: u32, hardware: bool }
fn mover_ray(s: u32, o: vec3f, d: vec3f, far: f32) -> MoverRay {
    let m=mover_enter(s,o,d);
    let surface=instance_surfaces[s+1u];
    let ro=vec4f(m.ro,1.0);
    let rd=vec4f(m.rd,0.0);
    let fo=vec3f(dot(surface.forward0,ro),dot(surface.forward1,ro),dot(surface.forward2,ro));
    let fd=vec3f(dot(surface.forward0,rd),dot(surface.forward1,rd),dot(surface.forward2,rd));
    let hardware=deform_steps<2u && (surface.info.x&SPRITE)==0u && rt_valid(fo,fd,far);
    if (!hardware) { mover_leave(); }
    return MoverRay(m,fo,fd,surface.info.x,hardware);
}
fn mover_desc(r: MoverRay, far: f32) -> RayDesc {
    return RayDesc(RAY_FLAG_FORCE_NO_OPAQUE,RT_MOVER_MASK,0.0,far,r.fo,r.fd);
}
fn mover_mine(c: RayIntersection, s: u32) -> bool {
    return c.kind==RAY_QUERY_INTERSECTION_TRIANGLE && c.instance_custom_data==s+1u;
}
const RT_BOXES_MASK: u32 = 128u;
const RT_WORLD: u32 = 0u;
const RT_OVERFLOW: u32 = 1u;
const RT_MOVER: u32 = 2u;
fn rt_keep(batch: ptr<function,array<u32,4>>, count: ptr<function,u32>, s: u32) -> bool {
    if (*count<4u) {
        (*batch)[*count]=s;
        *count+=1u;
        return false;
    }
    var top=0u;
    for (var j=1u;j<4u;j++) {
        if ((*batch)[j]>(*batch)[top]) { top=j; }
    }
    if (s<(*batch)[top]) { (*batch)[top]=s; }
    return true;
}
fn traverse(o: vec3f, d: vec3f, limit: f32, mode: u32) -> Traced {
    let closest=mode==QUERY_CLOSEST;
    trace_count(select(COUNT_SHADOW,COUNT_CLOSEST,closest));
    var traced=Traced(empty_hit(limit),vec3f(1.0));
    if (!rt_valid(o,d,limit)) {
        if (!closest) { traced.passed=vec3f(0.0); }
        return traced;
    }
    var rq: ray_query;
    var best=limit;
    var shape=0u;
    var mover=0u;
    var moved=traced.hit;
    var triangle=false;
    var committed_t=limit;
    var committed_at=vec2u(0u);
    var committed_uv=vec2f(0.0);
    var batch=array<u32,4>(0u,0u,0u,0u);
    var count=0u;
    var next=0u;
    var after=0u;
    var more=false;
    var phase=RT_WORLD;
    var current=0u;
    var ray=MoverRay(Moved(o,d,1.0),o,d,0u,false);
    var probe=traced.hit;
    var far=limit;
    var reach=limit;
    loop {
        if (phase==RT_MOVER) {
            trace_count(COUNT_MOVER_QUERIES);
            rayQueryInitialize(&rq,rt_movers,mover_desc(ray,far));
        } else {
            var desc=RayDesc(select(RAY_FLAG_TERMINATE_ON_FIRST_HIT,RAY_FLAG_NONE,closest),rt_mask(),0.0001,limit,o,d);
            if (phase==RT_OVERFLOW) {
                trace_count(COUNT_MOVER_QUERIES);
                desc=RayDesc(RAY_FLAG_NONE,RT_BOXES_MASK,0.0001,reach,o,d);
            }
            rayQueryInitialize(&rq,rt_world,desc);
        }
        while (rayQueryProceed(&rq)) {
            let c=rayQueryGetCandidateIntersection(&rq);
            if (phase==RT_MOVER) {
                if (!mover_mine(c,current)) { continue; }
                let i=rt_triangle(c);
                if (closest) {
                    tri_hit(i,current+1u,ray.word,ray.m.inv_det,ray.m.ro,ray.m.rd,o,d,false,&probe);
                    if (probe.t<far) {
                        far=probe.t;
                        rayQueryConfirmIntersection(&rq);
                    }
                } else if (tri_blocks(i,current+1u,ray.word,ray.m.inv_det,ray.m.ro,ray.m.rd,o,d,limit)) {
                    if (mode==QUERY_OCCLUDED) { traced.passed=vec3f(0.0); } else { traced.passed*=tri_tint(i,current+1u); }
                }
            } else {
                if (phase==RT_WORLD) { trace_count(select(COUNT_SHADOW_CANDIDATES,COUNT_CLOSEST_CANDIDATES,closest)); }
                if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
                    let record=rt_record(c);
                    let info=instance_surfaces[record].info;
                    let i=rt_triangle(c);
                    let ro=record_ray(record,o,1.0);
                    let rd=record_ray(record,d,0.0);
                    if (closest) {
                        var candidate=traced.hit;
                        candidate.t=best;
                        tri_hit(i,record,info.x,bitcast<f32>(info.w),ro,rd,o,d,false,&candidate);
                        if (candidate.t<best) {
                            rayQueryConfirmIntersection(&rq);
                            best=candidate.t;
                            shape=0u;
                            mover=0u;
                        }
                    } else if (tri_blocks(i,record,info.x,bitcast<f32>(info.w),ro,rd,o,d,limit)) {
                        if (mode==QUERY_OCCLUDED) { rayQueryConfirmIntersection(&rq); } else { traced.passed*=tri_tint(i,record); }
                    }
                } else if (TRACE_MOTION && c.instance_custom_data==RT_MOVERS) {
                    let s=mover_slot(c.primitive_index)+1u;
                    if (s>after) { more=rt_keep(&batch,&count,s) || more; }
                } else {
                    let i=rt_shape(c);
                    let t=shape_t(i,o,d,best);
                    if (t<best && t>0.0001) {
                        if (closest) {
                            best=t;
                            shape=i+1u;
                            mover=0u;
                        } else if (mode==QUERY_OCCLUDED) {
                            traced.passed=vec3f(0.0);
                        } else {
                            traced.passed*=cast_tint(u32(shapes[i].c.w));
                        }
                    }
                }
            }
            if (!closest && all(traced.passed<=vec3f(0.0))) { rayQueryTerminate(&rq); }
        }
        if (phase==RT_WORLD) {
            let c=rayQueryGetCommittedIntersection(&rq);
            if (!closest) {
                if (c.kind!=RAY_QUERY_INTERSECTION_NONE) { traced.passed=vec3f(0.0); }
            } else if (c.kind==RAY_QUERY_INTERSECTION_TRIANGLE) {
                triangle=true;
                committed_t=c.t;
                committed_at=vec2u(rt_triangle(c),rt_record(c));
                committed_uv=c.barycentrics;
                reach=min(best,c.t);
            } else {
                reach=best;
            }
        } else if (phase==RT_MOVER) {
            mover_leave();
            if (closest && probe.t<reach) {
                reach=probe.t;
                best=probe.t;
                moved=probe;
                mover=current+1u;
                shape=0u;
            }
        }
        var queued=false;
        loop {
            if (!closest && all(traced.passed<=vec3f(0.0))) { break; }
            if (next>=count) {
                if (more) {
                    for (var j=0u;j<count;j++) { after=max(after,batch[j]); }
                    count=0u;
                    next=0u;
                    more=false;
                    phase=RT_OVERFLOW;
                    queued=true;
                }
                break;
            }
            let s=batch[next]-1u;
            next+=1u;
            ray=mover_ray(s,o,d,reach);
            if (ray.hardware) {
                current=s;
                probe=empty_hit(reach);
                far=reach;
                phase=RT_MOVER;
                queued=true;
                break;
            }
            if (RT_WALK) {
                var walked=empty_hit(reach);
                let through=mover_walk(s,o,d,reach,mode,!closest,&walked);
                if (!closest) {
                    traced.passed*=through;
                } else if (walked.t<reach) {
                    reach=walked.t;
                    best=walked.t;
                    moved=walked;
                    mover=s+1u;
                    shape=0u;
                }
            }
        }
        if (!queued) { break; }
    }
    if (!closest) {
        if (!RT_BOXES && any(traced.passed>vec3f(0.0))) { shapes_through(o,d,limit,mode,&traced); }
        return traced;
    }
    if (shape>0u && !(triangle && committed_t<best)) {
        traced.hit=shape_hit(shape-1u,o,d,best);
    } else if (TRACE_MOTION && mover>0u && !(triangle && committed_t<best)) {
        traced.hit=moved;
    } else if (triangle) {
        tri_fill(committed_at.x,committed_at.y,committed_t,committed_uv.x,committed_uv.y,&traced.hit);
    }
    if (!RT_BOXES) { shapes_through(o,d,limit,mode,&traced); }
    traced.hit=world_hit(traced.hit);
    return traced;
}
