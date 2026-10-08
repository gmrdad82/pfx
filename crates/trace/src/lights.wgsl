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
    if (previous_pdf<=0.0 || all(materials[hit.material].emission_film.xyz<=vec3f(0.0))) { return 1.0; }
    var index=0u;
    if (hit.shape>0u) { index=u32(shapes[hit.shape-1u].d.y); } else { index=u32(triangles[hit.surface].c.w); }
    if (index==0u) { return 1.0; }
    return mis_power(previous_pdf,emitter_pdf(emitters[index],p,previous_point,hit.geometric));
}
fn sampled_emitter(hit: Hit) -> bool {
    if (all(materials[hit.material].emission_film.xyz<=vec3f(0.0))) { return false; }
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
fn emitter_candidate(transform: mat3x3f, p: vec3f, origin: vec3f, state: ptr<function,u32>) -> Candidate {
    let count=u32(emitters[0].a.x);
    let e=emitters[find_emitter(count,random(state))+1u];
    let out=candidate_none();
    if (e.a.w>0.5) {
        let to=e.a.xyz-p;
        let distance2=dot(to,to);
        let r2=e.c.x*e.c.x;
        let u1=random(state);
        let u2=random(state);
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
        if (local.z<=0.0) { return out; }
        return Candidate(local,e.emission.xyz,e.emission.w*cone_pdf(p,e.a.xyz,e.c.x),0.0,direction,0.0,u32(e.b.w)+1u);
    }
    let r1=sqrt(random(state));
    let r2=random(state);
    let x=e.a.xyz*(1.0-r1)+e.b.xyz*(r1*(1.0-r2))+e.c.xyz*(r1*r2);
    if (clipped(u32(e.b.w),x)) { return out; }
    let to=x-p;
    let distance2=dot(to,to);
    let direction=to/sqrt(max(distance2,1e-30));
    let local=transpose(transform)*direction;
    if (local.z<=0.0) { return out; }
    let normal=normalize(cross(e.b.xyz-e.a.xyz,e.c.xyz-e.a.xyz));
    if (abs(dot(normal,p-e.a.xyz))<=1e-5) { return out; }
    let cosine=abs(dot(normal,direction));
    if (cosine<=1e-7) { return out; }
    let ray=shadow_ray(origin,x,true);
    return Candidate(local,e.emission.xyz,e.emission.w/e.c.w*distance2/cosine,0.0,ray.xyz,ray.w,0u);
}
fn light_candidate(light: Light, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>) -> Candidate {
    let out=candidate_none();
    let shadow=light.d.w>0.5;
    let kind=u32(light.a.w);
    if (kind==2u || kind==3u) {
        let u1=random(state);
        let u2=random(state);
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
        if (local.z<=0.0) { return out; }
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
    if (local.z<=0.0 || dot(geometric,l)<=0.0) { return out; }
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
        let r=radius*sqrt(random(state));
        let phi=2.0*PI*random(state);
        aim+=disc[0]*(r*cos(phi))+disc[1]*(r*sin(phi));
    }
    let ratio=distance/range;
    let window=clamp(1.0-ratio*ratio*ratio*ratio,0.0,1.0);
    let reach=max(distance,radius);
    let ray=shadow_ray(origin,aim,shadow);
    let roughness=sqrt(clamp(radius/(2.0*distance),0.0,1.0));
    return Candidate(local,light.b.xyz*(window*window*cone/(reach*reach)),0.0,roughness,ray.xyz,ray.w,0u);
}
fn light_slots() -> u32 {
    return 1u+u32(emitters[0].a.y);
}
fn direct_light(slot: u32, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>) -> Candidate {
    if (slot>0u) { return light_candidate(lights[slot-1u],transform,p,origin,geometric,state); }
    if (emitters[0].a.x<0.5) { return candidate_none(); }
    return emitter_candidate(transform,p,origin,state);
}
