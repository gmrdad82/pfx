struct Emitter { a: vec4f, b: vec4f, c: vec4f, emission: vec4f }
struct Light { a: vec4f, b: vec4f, c: vec4f, d: vec4f }
@group(0) @binding(14) var<storage, read> emitters: array<Emitter>;
@group(0) @binding(15) var<storage, read> emitter_cdf: array<f32>;
@group(0) @binding(16) var<storage, read> lights: array<Light>;

fn mis_power(chosen: f32, other: f32) -> f32 {
    if (chosen<=0.0) { return 0.0; }
    let ratio=other/chosen;
    return 1.0/(1.0+ratio*ratio);
}
fn find_emitter(count: u32, u: f32) -> u32 {
    var low=0u;
    var high=count;
    loop {
        if (low>=high) { break; }
        let mid=(low+high)/2u;
        if (emitter_cdf[mid]<=u) { low=mid+1u; } else { high=mid; }
    }
    return min(low,count-1u);
}
fn cone_pdf(apex: vec3f, center: vec3f, radius: f32) -> f32 {
    let to=center-apex;
    let distance2=dot(to,to);
    let r2=radius*radius;
    if (distance2<=r2) { return 1.0/(4.0*PI); }
    let cos_max=sqrt(max(1.0-r2/distance2,0.0));
    let open=(r2/distance2)/(1.0+cos_max);
    return 1.0/(2.0*PI*open);
}
fn emitter_pdf(e: Emitter, x: vec3f, apex: vec3f, normal: vec3f) -> f32 {
    if (e.a.w>0.5) { return e.emission.w*cone_pdf(apex,e.a.xyz,e.c.x); }
    let to=x-apex;
    let distance2=dot(to,to);
    let cosine=abs(dot(normal,to))/sqrt(max(distance2,1e-30));
    return e.emission.w/max(e.c.w,1e-30)*distance2/max(cosine,1e-7);
}
fn emitter_weight(hit: Hit, p: vec3f, previous_point: vec3f, previous_pdf: f32) -> f32 {
    if (previous_pdf<=0.0 || all(material_row(hit.material,5u).xyz<=vec3f(0.0))) { return 1.0; }
    var index=0u;
    if (hit.shape>0u) { index=u32(shapes[hit.shape-1u].d.y); } else { index=u32(triangles[hit.surface].c.w); }
    if (index==0u) { return 1.0; }
    return mis_power(previous_pdf,emitter_pdf(emitters[index],p,previous_point,hit.geometric));
}
fn sampled_emitter(hit: Hit) -> bool {
    if (all(material_row(hit.material,5u).xyz<=vec3f(0.0))) { return false; }
    if (hit.shape>0u) { return u32(shapes[hit.shape-1u].d.y)>0u; }
    return u32(triangles[hit.surface].c.w)>0u;
}
fn clipped(slot: u32, point: vec3f) -> bool {
    let surface=instance_surfaces[slot];
    return (any(surface.clip0.xyz != vec3f(0.0)) && dot(surface.clip0.xyz,point)>surface.clip0.w)
        || (any(surface.clip1.xyz != vec3f(0.0)) && dot(surface.clip1.xyz,point)>surface.clip1.w);
}
fn rect_frame(light: Light) -> vec4f {
    let spanned=cross(light.c.xyz,light.d.xyz)*4.0;
    let area=length(spanned);
    return vec4f(spanned/area,area);
}
fn area_frame(light: Light) -> vec4f {
    if (u32(light.a.w)==3u) { return vec4f(light.c.xyz,PI*light.d.x*light.d.x); }
    return rect_frame(light);
}
fn light_hits(o: vec3f, d: vec3f, limit: f32, previous_point: vec3f, previous_pdf: f32) -> vec3f {
    let count=u32(emitters[0].a.y);
    var total=vec3f(0.0);
    for (var i=0u;i<count;i++) {
        let light=lights[i];
        let kind=u32(light.a.w);
        if ((kind!=2u && kind!=3u) || light.d.w<0.5) { continue; }
        let plane=area_frame(light);
        let facing=dot(d,plane.xyz);
        if (abs(facing)<1e-9 || (light.c.w<0.5 && facing>=0.0)) { continue; }
        let t=dot(light.a.xyz-o,plane.xyz)/facing;
        if (t<=0.0001 || t>=limit) { continue; }
        let x=o+d*t;
        let q=x-light.a.xyz;
        if (kind==3u) {
            if (dot(q,q)>light.d.x*light.d.x) { continue; }
        } else if (abs(dot(q,light.c.xyz))>dot(light.c.xyz,light.c.xyz) || abs(dot(q,light.d.xyz))>dot(light.d.xyz,light.d.xyz)) { continue; }
        var weight=1.0;
        if (previous_pdf>0.0) {
            let to=x-previous_point;
            let distance2=dot(to,to);
            let cosine=abs(dot(plane.xyz,to))/sqrt(max(distance2,1e-30));
            weight=mis_power(previous_pdf,distance2/(plane.w*max(cosine,1e-7)));
        }
        total+=light.b.xyz*weight;
    }
    return total;
}
fn candidate_none() -> Candidate {
    return Candidate(vec3f(0.0,0.0,1.0),vec3f(0.0),0.0,0.0,vec3f(0.0,0.0,1.0),-1.0,0u);
}
fn shadow_ray(origin: vec3f, aim: vec3f, shadow: bool) -> vec4f {
    if (!shadow) { return vec4f(0.0,0.0,1.0,0.0); }
    let ray=aim-origin;
    let distance=length(ray);
    let limit=distance*0.9999-0.0002;
    if (limit<=0.0) { return vec4f(0.0,0.0,1.0,0.0); }
    return vec4f(ray/distance,limit);
}
fn emitter_candidate(transform: mat3x3f, p: vec3f, origin: vec3f, state: ptr<function,u32>, u: vec4f) -> Candidate {
    let count=u32(emitters[0].a.x);
    let e=emitters[find_emitter(count,rnd(state,u.z))+1u];
    let out=candidate_none();
    if (e.a.w>0.5) {
        let to=e.a.xyz-p;
        let distance2=dot(to,to);
        let r2=e.c.x*e.c.x;
        let u1=rnd(state,u.x);
        let u2=rnd(state,u.y);
        var direction=vec3f(0.0);
        if (distance2<=r2) {
            let z=1.0-2.0*u1;
            let r=sqrt(max(0.0,1.0-z*z));
            direction=vec3f(r*cos(2.0*PI*u2),r*sin(2.0*PI*u2),z);
        } else {
            let cos_max=sqrt(max(1.0-r2/distance2,0.0));
            let open=(r2/distance2)/(1.0+cos_max);
            let cos_theta=1.0-u1*open;
            let sin_theta=sqrt(max(0.0,1.0-cos_theta*cos_theta));
            direction=normalize(basis(to/sqrt(distance2))*vec3f(cos(2.0*PI*u2)*sin_theta,sin(2.0*PI*u2)*sin_theta,cos_theta));
        }
        let local=transpose(transform)*direction;
        if (local.z<=0.0 && !volume_vertex) { return out; }
        return Candidate(local,e.emission.xyz,e.emission.w*cone_pdf(p,e.a.xyz,e.c.x),0.0,direction,0.0,u32(e.b.w)+1u);
    }
    let r1=sqrt(rnd(state,u.x));
    let r2=rnd(state,u.y);
    let x=e.a.xyz*(1.0-r1)+e.b.xyz*(r1*(1.0-r2))+e.c.xyz*(r1*r2);
    if (clipped(u32(e.b.w),x)) { return out; }
    let to=x-p;
    let distance2=dot(to,to);
    let direction=to/sqrt(max(distance2,1e-30));
    let local=transpose(transform)*direction;
    if (local.z<=0.0 && !volume_vertex) { return out; }
    let normal=normalize(cross(e.b.xyz-e.a.xyz,e.c.xyz-e.a.xyz));
    if (abs(dot(normal,p-e.a.xyz))<=1e-5) { return out; }
    let cosine=abs(dot(normal,direction));
    if (cosine<=1e-7) { return out; }
    let ray=shadow_ray(origin,x,true);
    return Candidate(local,e.emission.xyz,e.emission.w/e.c.w*distance2/cosine,0.0,ray.xyz,ray.w,0u);
}
fn light_candidate(light: Light, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>, u: vec4f) -> Candidate {
    let out=candidate_none();
    let shadow=light.d.w>0.5;
    let kind=u32(light.a.w);
    if (kind==2u || kind==3u) {
        let u1=rnd(state,u.x);
        let u2=rnd(state,u.y);
        let plane=area_frame(light);
        var x=light.a.xyz+light.c.xyz*(2.0*u1-1.0)+light.d.xyz*(2.0*u2-1.0);
        if (kind==3u) {
            let disc=basis(light.c.xyz);
            let r=light.d.x*sqrt(u1);
            let phi=2.0*PI*u2;
            x=light.a.xyz+disc[0]*(r*cos(phi))+disc[1]*(r*sin(phi));
        }
        let to=x-p;
        let distance2=dot(to,to);
        let direction=to/sqrt(max(distance2,1e-30));
        let local=transpose(transform)*direction;
        if (local.z<=0.0 && !volume_vertex) { return out; }
        let facing=-dot(plane.xyz,direction);
        let cosine=select(facing,abs(facing),light.c.w>0.5);
        if (cosine<=1e-7) { return out; }
        let pdf=distance2/(plane.w*cosine);
        let ray=shadow_ray(origin,x,shadow);
        if (!shadow) { return Candidate(local,light.b.xyz/pdf,0.0,0.0,ray.xyz,ray.w,0u); }
        return Candidate(local,light.b.xyz,pdf,0.0,ray.xyz,ray.w,0u);
    }
    let to=light.a.xyz-p;
    let distance=length(to);
    let range=light.b.w;
    if (distance>=range || distance<=1e-6) { return out; }
    let l=to/distance;
    let local=transpose(transform)*l;
    if (!volume_vertex && (local.z<=0.0 || dot(geometric,l)<=0.0)) { return out; }
    var cone=1.0;
    if (u32(light.a.w)==1u) {
        let along=clamp(dot(-l,light.c.xyz)*light.d.x+light.d.y,0.0,1.0);
        cone=along*along;
        if (cone<=0.0) { return out; }
    }
    let radius=light.c.w;
    var aim=light.a.xyz;
    if (shadow && radius>0.0) {
        let disc=basis(l);
        let r=radius*sqrt(rnd(state,u.x));
        let phi=2.0*PI*rnd(state,u.y);
        aim+=disc[0]*(r*cos(phi))+disc[1]*(r*sin(phi));
    }
    let ratio=distance/range;
    let window=clamp(1.0-ratio*ratio*ratio*ratio,0.0,1.0);
    let reach=max(distance,radius);
    let ray=shadow_ray(origin,aim,shadow);
    let roughness=sqrt(clamp(radius/(2.0*distance),0.0,1.0));
    return Candidate(local,light.b.xyz*(window*window*cone/(reach*reach)),0.0,roughness,ray.xyz,ray.w,0u);
}
fn sin_from_cos(c: f32) -> f32 {
    return sqrt(max(1.0-c*c,0.0));
}
fn tree_importance(n: vec3f, to: vec3f, cos_u: f32, axis: vec3f, theta_o: f32, theta_e: f32, far: f32, near: f32, energy: f32) -> vec2f {
    let sin_u=sin_from_cos(cos_u);
    let cos_i=dot(to,n);
    let sin_i=sin_from_cos(cos_i);
    var cos_min_i=select(cos_i*cos_u+sin_i*sin_u,1.0,cos_i>=cos_u);
    if (cos_min_i<0.0 && !volume_vertex) { return vec2f(0.0); }
    var cos_max_i=max(cos_i*cos_u-sin_i*sin_u,0.0);
    if (volume_vertex) {
        cos_min_i=1.0;
        cos_max_i=1.0;
    }
    var cos_t=1.0;
    var sin_t=0.0;
    if (any(axis!=-to)) {
        cos_t=dot(axis,-to);
        sin_t=sin_from_cos(cos_t);
    }
    let cos_t_minus_u=cos_t*cos_u+sin_t*sin_u;
    let cos_o=cos(theta_o);
    let sin_o=sin(theta_o);
    var cos_min_out=1.0;
    if (cos_t>=cos_u || cos_t_minus_u>=cos_o) {
        cos_min_out=1.0;
    } else if (theta_o+theta_e>PI || cos_t_minus_u>cos(theta_o+theta_e)) {
        cos_min_out=cos_t_minus_u*cos_o+sin_from_cos(cos_t_minus_u)*sin_o;
    } else {
        return vec2f(0.0);
    }
    let most=abs(cos_min_i*energy*cos_min_out/(near*near));
    let cos_t_plus_u=cos_t*cos_u-sin_t*sin_u;
    if (theta_e-theta_o<0.0 || cos_t<0.0 || cos_u<0.0 || cos_t_plus_u<cos(theta_e-theta_o)) { return vec2f(most,0.0); }
    let cos_max_out=cos_t_plus_u*cos_o-sin_from_cos(cos_t_plus_u)*sin_o;
    return vec2f(most,abs(cos_max_i*energy*cos_max_out/(far*far)));
}
fn node_importance(node: Light, p: vec3f, n: vec3f) -> vec2f {
    let centroid=0.5*(node.a.xyz+node.b.xyz);
    let extent=node.b.xyz-centroid;
    if (!volume_vertex && dot(n,centroid-p)+dot(abs(n),abs(extent))<=0.0) { return vec2f(0.0); }
    if (length(p-clamp(p,node.a.xyz,node.b.xyz))>=node.b.w) { return vec2f(0.0); }
    let to=centroid-p;
    let reach=length(to);
    let radius2=dot(extent,extent);
    var cos_u=-1.0;
    if (reach*reach>radius2) { cos_u=sqrt(max(1.0-radius2/(reach*reach),0.0)); }
    let distance=max(max(0.5*sqrt(radius2),reach),1e-6);
    return tree_importance(n,to/max(reach,1e-30),cos_u,node.c.xyz,node.c.w,node.d.x,distance,distance,node.a.w);
}
fn spot_cone(light: Light, cosine: f32) -> f32 {
    let along=clamp(cosine*light.d.x+light.d.y,0.0,1.0);
    return along*along;
}
fn emitter_importance(light: Light, p: vec3f, n: vec3f) -> vec2f {
    let to=light.a.xyz-p;
    let near=length(to);
    if (near>=light.b.w || near<=1e-6) { return vec2f(0.0); }
    let toward=to/near;
    let radius=light.c.w;
    let far=sqrt(radius*radius+near*near);
    let cos_u=near/far;
    var energy=(light.b.x+light.b.y+light.b.z)/3.0;
    if (u32(light.a.w)==1u) {
        let theta_e=acos(clamp(-light.d.y/light.d.x,-1.0,1.0));
        let cos_min_out=cos(max(0.0,acos(clamp(dot(light.c.xyz,-toward),-1.0,1.0))-acos(cos_u)));
        energy*=spot_cone(light,cos_min_out);
        return tree_importance(n,toward,cos_u,light.c.xyz,0.0,theta_e,far,near,energy);
    }
    return tree_importance(n,toward,cos_u,-toward,0.0,PI/2.0,far,near,energy);
}
fn child_importance(node: Light, p: vec3f, n: vec3f) -> vec2f {
    if (u32(node.d.y)==1u) { return emitter_importance(lights[u32(node.d.z)],p,n); }
    return node_importance(node,p,n);
}
fn tree_leaf(first: u32, count: u32, p: vec3f, n: vec3f, start: f32) -> vec2f {
    var rand=start;
    let sample_max=rand>0.5;
    if (count>1u) { rand=rand*2.0-select(0.0,1.0,sample_max); }
    var selected=-1;
    var chosen=0.0;
    var chosen_other=0.0;
    var total=0.0;
    var total_other=0.0;
    var has=0u;
    for (var i=0u;i<count;i++) {
        let index=i32(first+i);
        let importance=emitter_importance(lights[first+i],p,n);
        let weight=select(importance.y,importance.x,sample_max);
        let other=select(importance.x,importance.y,sample_max);
        if (weight>0.0) {
            total+=weight;
            if (selected<0) {
                selected=index;
                chosen=weight;
            } else {
                let thresh=weight/total;
                if (rand<=thresh) {
                    selected=index;
                    chosen=weight;
                    rand=rand/thresh;
                } else {
                    rand=(rand-thresh)/(1.0-thresh);
                }
                rand=clamp(rand,0.0,1.0);
            }
        }
        if (selected==index) { chosen_other=other; }
        total_other+=other;
        has|=select(0u,1u,importance.x>0.0)<<i;
    }
    if (has==0u) { return vec2f(0.0); }
    var chosen_max=select(chosen_other,chosen,sample_max);
    var total_max=select(total_other,total,sample_max);
    var chosen_min=select(chosen,chosen_other,sample_max);
    var total_min=select(total,total_other,sample_max);
    if (total_min==0.0) {
        if (sample_max) {
            chosen_min=1.0;
            total_min=f32(countOneBits(has));
        } else {
            selected=-1;
            var valid=0.0;
            var bits=has;
            for (var i=0u;i<count;i++) {
                if ((bits&1u)!=0u) {
                    valid+=1.0;
                    if (selected<0) {
                        selected=i32(first+i);
                    } else {
                        let thresh=1.0/valid;
                        if (rand<=thresh) {
                            selected=i32(first+i);
                            rand=rand/thresh;
                        } else {
                            rand=(rand-thresh)/(1.0-thresh);
                        }
                        rand=clamp(rand,0.0,1.0);
                    }
                }
                bits>>=1u;
            }
            chosen_min=1.0;
            total_min=valid;
            chosen_max=emitter_importance(lights[u32(selected)],p,n).x;
        }
    }
    return vec2f(f32(selected),0.5*(chosen_max/total_max+chosen_min/total_min));
}
fn tree_pick(p: vec3f, n: vec3f, start: f32) -> vec2f {
    var at=u32(emitters[0].a.y+emitters[0].a.w);
    var rand=start;
    var pdf=1.0;
    loop {
        let node=lights[at];
        let count=u32(node.d.y);
        if (count>0u) {
            let leaf=tree_leaf(u32(node.d.z),count,p,n,rand);
            return vec2f(leaf.x,leaf.y*pdf);
        }
        let left=at+1u;
        let right=u32(node.d.z);
        let l=child_importance(lights[left],p,n);
        let r=child_importance(lights[right],p,n);
        let total_max=l.x+r.x;
        if (total_max==0.0) { return vec2f(0.0); }
        let total_min=l.y+r.y;
        var probability_min=0.5*(select(0.0,1.0,l.x>0.0)+select(0.0,1.0,r.x==0.0));
        if (total_min>0.0) { probability_min=l.y/total_min; }
        let left_probability=0.5*(l.x/total_max+probability_min);
        let right_weight=1.0-left_probability;
        at=left;
        if (right_weight>0.0) {
            let thresh=right_weight/(left_probability+right_weight);
            if (rand<=thresh) {
                at=right;
                rand=rand/thresh;
            } else {
                rand=(rand-thresh)/(1.0-thresh);
            }
            rand=clamp(rand,0.0,1.0);
        }
        pdf*=select(1.0-left_probability,left_probability,at==left);
    }
    return vec2f(0.0);
}
fn tree_light(transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>, u: vec4f) -> Candidate {
    let picked=tree_pick(p,transform[2],rnd(state,u.z));
    if (!(picked.y>0.0)) { return candidate_none(); }
    var candidate=light_candidate(lights[u32(picked.x)],transform,p,origin,geometric,state,u);
    candidate.scale/=picked.y;
    return candidate;
}
fn light_position(i: u32) -> vec3f {
    return lights[i].a.xyz;
}
fn light_slots() -> u32 {
    return 1u+u32(emitters[0].a.y)+select(0u,1u,emitters[0].a.w>0.5);
}
fn direct_light(slot: u32, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>, u: vec4f) -> Candidate {
    let looped=u32(emitters[0].a.y);
    if (slot>looped) { return tree_light(transform,p,origin,geometric,state,u); }
    if (slot>0u) { return light_candidate(lights[slot-1u],transform,p,origin,geometric,state,u); }
    if (emitters[0].a.x<0.5) { return candidate_none(); }
    return emitter_candidate(transform,p,origin,state,u);
}
fn emitters_sampled() -> bool {
    return emitters[0].a.x>0.5;
}
fn volume_light(index: u32, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>, u: vec4f) -> Candidate {
    if (index==VOLUME_EMITTERS) {
        if (emitters[0].a.x<0.5) { return candidate_none(); }
        return emitter_candidate(transform,p,origin,state,u);
    }
    return light_candidate(lights[index],transform,p,origin,geometric,state,u);
}
fn segment_slots() -> u32 {
    return u32(emitters[0].a.y)+select(0u,1u,emitters[0].a.w>0.5);
}
fn segment_theta(closest: f32, distance: f32, t: f32) -> f32 {
    return atan2(t,distance-closest*select(0.0,(t-closest)/distance,distance!=0.0));
}
fn segment_v(centroid: vec3f, p: vec3f, d: vec3f, axis: vec3f, t: f32) -> vec3f {
    let u0=p-centroid;
    let u1=u0+d*min(t,1e12);
    let v0=select(d,u0*inverseSqrt(dot(u0,u0)),dot(u0,u0)>1e-30);
    let v1=select(-d,u1*inverseSqrt(dot(u1,u1)),dot(u1,u1)>1e-30);
    let side=cross(v0,v1);
    let s=length(side);
    if (!(s>1e-9)) { return select(v1,v0,dot(v0,axis)>dot(v1,axis)); }
    let o1=cross(side/s,v0);
    let a0=dot(v0,axis);
    let a1=dot(o1,axis);
    let span=a0*a0+a1*a1;
    if (!(span>0.0)) { return v0; }
    let inv=inverseSqrt(span);
    let cos_phi0=a0*inv;
    if (a1<0.0 || dot(v0,v1)>cos_phi0) { return select(v1,v0,a0>dot(v1,axis)); }
    return cos_phi0*v0+a1*inv*o1;
}
fn segment_outgoing(v: vec3f, cos_u: f32, axis: vec3f, theta_o: f32, theta_e: f32) -> f32 {
    var cos_t=1.0;
    var sin_t=0.0;
    if (any(axis!=v)) {
        cos_t=dot(axis,v);
        sin_t=sin_from_cos(cos_t);
    }
    let sin_u=sin_from_cos(cos_u);
    let cos_t_minus_u=cos_t*cos_u+sin_t*sin_u;
    let cos_o=cos(theta_o);
    if (cos_t>=cos_u || cos_t_minus_u>=cos_o) { return 1.0; }
    if (theta_o+theta_e>PI || cos_t_minus_u>cos(theta_o+theta_e)) {
        return abs(cos_t_minus_u*cos_o+sin_from_cos(cos_t_minus_u)*sin(theta_o));
    }
    return 0.0;
}
fn segment_emitter(light: Light, p: vec3f, d: vec3f, t: f32) -> f32 {
    let centroid=light.a.xyz;
    let closest=dot(centroid-p,d);
    let line=length(centroid-p-d*closest);
    let at=p+d*clamp(closest,0.0,t);
    let near=max(length(centroid-at),1e-6);
    if (near>=light.b.w) { return 0.0; }
    let theta_d=segment_theta(closest,line,t);
    let kind=u32(light.a.w);
    var energy=(light.b.x+light.b.y+light.b.z)/3.0;
    if (kind==0u) { return energy*theta_d/near; }
    var axis=light.c.xyz;
    var theta_o=0.0;
    var theta_e=PI/2.0;
    var cos_u=1.0;
    if (kind==1u) {
        theta_e=acos(clamp(-light.d.y/light.d.x,-1.0,1.0));
        cos_u=near/sqrt(light.c.w*light.c.w+near*near);
    } else {
        let plane=area_frame(light);
        axis=plane.xyz;
        energy*=plane.w;
        if (light.c.w>0.5) { theta_o=PI; }
        var u_axis=light.c.xyz;
        var v_axis=light.d.xyz;
        if (kind==3u) {
            let disc=basis(light.c.xyz);
            u_axis=disc[0]*light.d.x;
            v_axis=disc[1]*light.d.x;
        }
        let toward=(centroid-at)/near;
        for (var i=0u;i<4u;i++) {
            let corner=centroid+u_axis*select(-1.0,1.0,(i&1u)!=0u)+v_axis*select(-1.0,1.0,(i&2u)!=0u);
            cos_u=min(cos_u,dot(toward,normalize(corner-at)));
        }
    }
    let outgoing=segment_outgoing(segment_v(centroid,p,d,axis,t),cos_u,axis,theta_o,theta_e);
    return energy*outgoing*theta_d/near;
}
fn segment_node(node: Light, p: vec3f, d: vec3f, t: f32) -> f32 {
    let centroid=0.5*(node.a.xyz+node.b.xyz);
    let extent=node.b.xyz-centroid;
    let radius2=dot(extent,extent);
    let closest=dot(centroid-p,d);
    let line=length(centroid-p-d*closest);
    let at=p+d*clamp(closest,0.0,t);
    let reach2=dot(centroid-at,centroid-at);
    if (sqrt(reach2)-sqrt(radius2)>=node.b.w) { return 0.0; }
    let theta_d=segment_theta(closest,line,t);
    var cos_u=-1.0;
    if (reach2>radius2) { cos_u=sqrt(max(1.0-radius2/reach2,0.0)); }
    let distance=max(max(0.5*sqrt(radius2),line),1e-6);
    let outgoing=segment_outgoing(segment_v(centroid,p,d,node.c.xyz,t),cos_u,node.c.xyz,node.c.w,node.d.x);
    return node.a.w*outgoing*theta_d/distance;
}
fn segment_child(node: Light, p: vec3f, d: vec3f, t: f32) -> f32 {
    if (u32(node.d.y)==1u) { return segment_emitter(lights[u32(node.d.z)],p,d,t); }
    return segment_node(node,p,d,t);
}
fn segment_root() -> u32 {
    return u32(emitters[0].a.y+emitters[0].a.w);
}
fn segment_importance(slot: u32, p: vec3f, d: vec3f, t: f32) -> f32 {
    if (slot<u32(emitters[0].a.y)) { return segment_emitter(lights[slot],p,d,t); }
    return segment_child(lights[segment_root()],p,d,t);
}
fn segment_leaf(first: u32, count: u32, p: vec3f, d: vec3f, t: f32, start: f32) -> vec2f {
    let by_weight=start>0.5;
    var rand=start;
    if (count>1u) { rand=rand*2.0-select(0.0,1.0,by_weight); }
    var selected=-1;
    var chosen=0.0;
    var total=0.0;
    var valid=0.0;
    for (var i=0u;i<count;i++) {
        let weight=segment_emitter(lights[first+i],p,d,t);
        if (!(weight>0.0)) { continue; }
        total+=weight;
        valid+=1.0;
        let thresh=select(1.0/valid,weight/total,by_weight);
        if (rand<=thresh) {
            selected=i32(first+i);
            chosen=weight;
            rand=rand/thresh;
        } else {
            rand=(rand-thresh)/(1.0-thresh);
        }
        rand=clamp(rand,0.0,1.0);
    }
    if (selected<0) { return vec2f(0.0); }
    return vec2f(f32(selected),0.5*(chosen/total+1.0/valid));
}
fn segment_tree(p: vec3f, d: vec3f, t: f32, start: f32) -> vec2f {
    var at=segment_root();
    var rand=start;
    var pdf=1.0;
    loop {
        let node=lights[at];
        let count=u32(node.d.y);
        if (count>0u) {
            let leaf=segment_leaf(u32(node.d.z),count,p,d,t,rand);
            return vec2f(leaf.x,leaf.y*pdf);
        }
        let left=at+1u;
        let right=u32(node.d.z);
        let l=segment_child(lights[left],p,d,t);
        let r=segment_child(lights[right],p,d,t);
        let total=l+r;
        if (!(total>0.0)) { return vec2f(0.0); }
        let probability_min=0.5*(select(0.0,1.0,l>0.0)+select(0.0,1.0,r==0.0));
        let left_probability=0.5*(l/total+probability_min);
        let right_weight=1.0-left_probability;
        at=left;
        if (right_weight>0.0) {
            let thresh=right_weight;
            if (rand<=thresh) {
                at=right;
                rand=rand/thresh;
            } else {
                rand=(rand-thresh)/(1.0-thresh);
            }
            rand=clamp(rand,0.0,1.0);
        }
        pdf*=select(1.0-left_probability,left_probability,at==left);
    }
    return vec2f(0.0);
}
fn segment_light(slot: u32, p: vec3f, d: vec3f, t: f32, start: f32) -> vec2f {
    if (slot<u32(emitters[0].a.y)) { return vec2f(f32(slot),1.0); }
    return segment_tree(p,d,t,start);
}
