struct Frame {
    size: vec4u,
    counts: vec4u,
    origin: vec4f,
    forward: vec4f,
    right: vec4f,
    up: vec4f,
    sun_dir: vec4f,
    sun_color: vec4f,
    lens: vec4f,
}
override TOTAL_BOUNCES: u32 = 16u;
override DIFFUSE_BOUNCES: u32 = 4u;
override GLOSSY_BOUNCES: u32 = 8u;
override TRANSMISSION_BOUNCES: u32 = 12u;
override BLOCK_STRIDE: u32 = 1u;
override TRANSMISSIVE_SHADOWS: bool = false;
override CLAMP_INDIRECT: f32 = 0.0;
override FILTER_GLOSSY: f32 = 0.0;
override STEAM_SPREAD_BASE: f32 = 0.7;
override STEAM_SPREAD_GROWTH: f32 = 4.0;
override STEAM_DRIFT_X_QUAD: f32 = -0.9;
override STEAM_DRIFT_X_LIN: f32 = -0.12;
override STEAM_DRIFT_Z_QUAD: f32 = 0.0;
override STEAM_DRIFT_Z_LIN: f32 = 0.02;
override STEAM_SWAY_X_WAVE: f32 = 17.0;
override STEAM_SWAY_X_RATE: f32 = 1.3;
override STEAM_SWAY_X_AMP: f32 = 0.012;
override STEAM_SWAY_X_BASE: f32 = 0.4;
override STEAM_SWAY_X_GROWTH: f32 = 6.0;
override STEAM_SWAY_Z_WAVE: f32 = 13.0;
override STEAM_SWAY_Z_RATE: f32 = 0.9;
override STEAM_SWAY_Z_AMP: f32 = 0.01;
override STEAM_SWAY_Z_BASE: f32 = 0.4;
override STEAM_SWAY_Z_GROWTH: f32 = 5.0;
override STEAM_FADE_IN: f32 = 0.02;
override STEAM_FADE_OUT_START: f32 = 0.07;
override STEAM_FADE_OUT_END: f32 = 0.2;
override STEAM_GRAIN_H: f32 = 70.0;
override STEAM_GRAIN_V: f32 = 42.0;
override STEAM_LIFT_RATE: f32 = 0.05;
override STEAM_WARP_AMP: f32 = 2.2;
override STEAM_WARP_RATE: f32 = 0.2;
override STEAM_CHURN: f32 = 0.7;
override STEAM_THRESHOLD_LOW: f32 = 0.42;
override STEAM_THRESHOLD_HIGH: f32 = 0.9;
struct Tri { a: vec4f, b: vec4f, c: vec4f, na: vec4f, nb: vec4f, nc: vec4f }
struct Node { lo: vec4f, hi: vec4f, first: u32, count: u32, pad: vec2u }
struct Shape { a: vec4f, b: vec4f, c: vec4f, d: vec4f }
struct Material {
    base_roughness: vec4f,
    metal_spec_coat: vec4f,
    sheen_trans_ior_disp: vec4f,
    thick_sub_film: vec4f,
    tint_absorption: vec4f,
    emission_film: vec4f,
    normal_maps: vec4f,
    maps_content: vec4f,
    age_a: vec4f,
    age_b: vec4f,
    layer0: vec4f,
    layer1: vec4f,
    layer2: vec4f,
    layer3: vec4f,
    tail: vec4f,
    params: array<vec4f, 16>,
}
struct Hit { t: f32, n: vec3f, geometric: vec3f, material: u32, uv: vec2f, tangent: vec3f, bitangent: vec3f, surface: u32, shape: u32 }
struct ContentInfo { offset: u32, width: u32, height: u32, kind: u32 }
struct InstanceSurface { clip0: vec4f, clip1: vec4f, transform: vec4f, crop: vec4f }
struct ContentLayer { layer: vec4f, extra: vec4f }
struct LightSample { dir: vec3f, pdf: f32, channel: u32, weight: vec3f, lobe: u32 }
struct Accum { color: vec4f, albedo: vec4f, normal: vec4f }
struct DetailResult { shade: Shaded, normal: vec3f }
struct Candidate { wi: vec3f, scale: vec3f, pdf: f32, roughness: f32, direction: vec3f, limit: f32, shape: u32 }
@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> triangles: array<Tri>;
@group(0) @binding(2) var<storage, read> nodes: array<Node>;
@group(0) @binding(3) var<storage, read> shapes: array<Shape>;
@group(0) @binding(4) var<storage, read> materials: array<Material>;
@group(0) @binding(5) var<storage, read> triangle_uv_v: array<vec4f>;
@group(0) @binding(6) var<storage, read> content_texels: array<vec4f>;
@group(0) @binding(7) var<storage, read_write> accum: array<Accum>;
@group(0) @binding(8) var output: texture_storage_2d<rgba32float, write>;
@group(0) @binding(9) var albedo: texture_storage_2d<rgba32float, write>;
@group(0) @binding(10) var normal: texture_storage_2d<rgba32float, write>;
@group(0) @binding(11) var<storage, read> content_info: array<ContentInfo>;
@group(0) @binding(12) var<storage, read> instance_surfaces: array<InstanceSurface>;
@group(0) @binding(13) var<storage, read> content_layers: array<ContentLayer>;

fn effects() -> Material {
    return materials[arrayLength(&materials)-1u];
}

fn trace_noise_start() {}
fn pcg(v: u32) -> u32 {
    let s = v * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}
fn hash3_xor(p: vec3i) -> f32 {
    let h = u32(p.x) * 73856093u ^ u32(p.y) * 19349663u ^ u32(p.z) * 83492791u;
    return f32(pcg(h) >> 8u) / 16777216.0;
}
fn hash3_nested(p: vec3i) -> f32 {
    let h = pcg(u32(p.x) * 73856093u ^ pcg(u32(p.y) * 19349663u ^ pcg(u32(p.z) * 83492791u)));
    return f32(h >> 8u) / 16777216.0;
}
fn value_noise3(p: vec3f, nested: bool) -> f32 {
    let i=vec3i(floor(p));
    let f=fract(p);
    let u=f*f*(3.0-2.0*f);
    var a0: f32;
    var a1: f32;
    var a2: f32;
    var a3: f32;
    var b0: f32;
    var b1: f32;
    var b2: f32;
    var b3: f32;
    if (nested) {
        a0=hash3_nested(i);
        a1=hash3_nested(i+vec3i(1,0,0));
        a2=hash3_nested(i+vec3i(0,1,0));
        a3=hash3_nested(i+vec3i(1,1,0));
        b0=hash3_nested(i+vec3i(0,0,1));
        b1=hash3_nested(i+vec3i(1,0,1));
        b2=hash3_nested(i+vec3i(0,1,1));
        b3=hash3_nested(i+vec3i(1,1,1));
    } else {
        a0=hash3_xor(i);
        a1=hash3_xor(i+vec3i(1,0,0));
        a2=hash3_xor(i+vec3i(0,1,0));
        a3=hash3_xor(i+vec3i(1,1,0));
        b0=hash3_xor(i+vec3i(0,0,1));
        b1=hash3_xor(i+vec3i(1,0,1));
        b2=hash3_xor(i+vec3i(0,1,1));
        b3=hash3_xor(i+vec3i(1,1,1));
    }
    let a=mix(mix(a0,a1,u.x),mix(a2,a3,u.x),u.y);
    let b=mix(mix(b0,b1,u.x),mix(b2,b3,u.x),u.y);
    return mix(a,b,u.z);
}
fn trace_noise_end() {}

fn random(state: ptr<function,u32>) -> f32 {
    *state = pcg(*state);
    return f32(*state >> 8u) / 16777216.0;
}
fn box_hit(node: Node, o: vec3f, d: vec3f, limit: f32) -> bool {
    var near = 0.0;
    var far = limit;
    for (var axis=0u; axis<3u; axis++) {
        if (abs(d[axis]) < 1e-12) {
            if (o[axis] < node.lo[axis] || o[axis] > node.hi[axis]) { return false; }
        } else {
            let a = (node.lo[axis]-o[axis])/d[axis];
            let b = (node.hi[axis]-o[axis])/d[axis];
            near = max(near,min(a,b));
            far = min(far,max(a,b));
            if (near > far) { return false; }
        }
    }
    return true;
}
const ONE_SIDED: u32 = 8388608u;
const LAYER_MASK: u32 = 262143u;
fn hit_layer(i: u32, material: u32) -> ContentLayer {
    let layer=(u32(triangles[i].b.w)>>5u) & LAYER_MASK;
    if (layer>0u) { return content_layers[layer-1u]; }
    return content_layers[material];
}
fn cut_out(i: u32, u: f32, v: f32) -> bool {
    let tri = triangles[i];
    if ((u32(tri.b.w)&4u)==0u) { return false; }
    let slot=i32(round(hit_layer(i,u32(tri.a.w)).layer.x));
    if (slot<0) { return true; }
    let uv_v=triangle_uv_v[i];
    let uv=vec2f(tri.na.w*(1.0-u-v)+tri.nb.w*u+tri.nc.w*v,uv_v.x*(1.0-u-v)+uv_v.y*u+uv_v.z*v);
    let surface=instance_surfaces[i];
    let at=content_uv(uv,surface.transform);
    if (!content_inside(at,surface.crop)) { return true; }
    let cutoff=uv_v.w;
    if (cutoff==0.5) { return content_at(slot,at).a<0.5; }
    return content_at(slot,at).a<cutoff;
}
fn tri_hit(i: u32, o: vec3f, d: vec3f, shadow_ray: bool, hit: ptr<function,Hit>) {
    let tri = triangles[i];
    let flags = u32(tri.b.w);
    if ((flags&1u)!=0u && !shadow_ray) { return; }
    if ((flags&2u)!=0u && shadow_ray) { return; }
    let e1 = tri.b.xyz-tri.a.xyz;
    let e2 = tri.c.xyz-tri.a.xyz;
    let p = cross(d,e2);
    let det = dot(e1,p);
    if (abs(det)<1e-9) { return; }
    if (!shadow_ray && (flags & ONE_SIDED) != 0u && det < 0.0) { return; }
    let inv = 1.0/det;
    let s = o-tri.a.xyz;
    let u = dot(s,p)*inv;
    if (u<0.0 || u>1.0) { return; }
    let q = cross(s,e1);
    let v = dot(d,q)*inv;
    if (v<0.0 || u+v>1.0) { return; }
    let t = dot(e2,q)*inv;
    if (t>0.0001 && t<(*hit).t) {
        let surface=instance_surfaces[i];
        let point=o+d*t;
        if ((any(surface.clip0.xyz != vec3f(0.0)) && dot(surface.clip0.xyz,point)>surface.clip0.w)
            || (any(surface.clip1.xyz != vec3f(0.0)) && dot(surface.clip1.xyz,point)>surface.clip1.w)) { return; }
        if (cut_out(i,u,v)) { return; }
        (*hit).t=t;
        (*hit).surface=i;
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
}
fn ellipsoid(p: vec3f, radius: vec3f) -> f32 {
    let k0 = length(p/radius);
    let k1 = length(p/(radius*radius));
    return k0*(k0-1.0)/max(k1,1e-8);
}
fn cone(p: vec3f, a: vec3f, b: vec3f, r1: f32, r2: f32) -> f32 {
    let ba=b-a;
    let length_axis=length(ba);
    if (length_axis<=abs(r1-r2)) {
        return select(length(p-b)-r2,length(p-a)-r1,r1>=r2);
    }
    let l2=dot(ba,ba);
    let rr=r1-r2;
    let a2=l2-rr*rr;
    let pa=p-a;
    let y=dot(pa,ba);
    let z=y-l2;
    let xv=pa*l2-ba*y;
    let x2=dot(xv,xv);
    let y2=y*y*l2;
    let z2=z*z*l2;
    let k=sign(rr)*rr*rr*x2;
    if (sign(z)*a2*z2>k) { return sqrt(x2+z2)/l2-r2; }
    if (sign(y)*a2*y2<k) { return sqrt(x2+y2)/l2-r1; }
    return sqrt(max(x2*a2/l2,0.0))/l2+y*rr/l2-r1;
}
fn base_distance(i: u32, p: vec3f) -> f32 {
    let s=shapes[i];
    let kind=u32(s.a.w);
    if (kind==1u) {
        let q=abs(p-s.a.xyz)-(s.b.xyz-vec3f(s.b.w));
        return length(max(q,vec3f(0.0)))+min(max(q.x,max(q.y,q.z)),0.0)-s.b.w;
    }
    if (kind==2u) { return cone(p,s.a.xyz,s.b.xyz,s.c.x,s.c.y); }
    if (kind==3u) { return ellipsoid(p-s.a.xyz,s.b.xyz); }
    if (kind==4u) {
        let ba=s.b.xyz-s.a.xyz;
        let h=clamp(dot(p-s.a.xyz,ba)/max(dot(ba,ba),1e-9),0.0,1.0);
        return length(p-s.a.xyz-ba*h)-s.c.x;
    }
    return 1e30;
}
fn shape_distance(i: u32, p: vec3f) -> f32 {
    let s=shapes[i];
    if (u32(s.a.w)==5u) {
        let a=base_distance(u32(s.b.x),p);
        let b=base_distance(u32(s.b.y),p);
        let radius=max(s.b.z,1e-6);
        let h=clamp(0.5+0.5*(b-a)/radius,0.0,1.0);
        return mix(b,a,h)-radius*h*(1.0-h);
    }
    return base_distance(i,p);
}
fn roots(a: f32, b: f32, c: f32) -> vec2f {
    if (abs(a)<1e-10) {
        if (abs(b)<1e-10) { return vec2f(1e30); }
        return vec2f(-c/(2.0*b),1e30);
    }
    let disc=b*b-a*c;
    if (disc<0.0) { return vec2f(1e30); }
    let root=sqrt(disc);
    return vec2f((-b-root)/a,(-b+root)/a);
}
fn consider(i: u32, o: vec3f, d: vec3f, t: f32, best: ptr<function,f32>) {
    if (t>0.0001 && t<*best && abs(shape_distance(i,o+d*t))<0.002) { *best=t; }
}
fn primitive_hit(i: u32, o: vec3f, d: vec3f, limit: f32) -> f32 {
    let shape=shapes[i];
    let kind=u32(shape.a.w);
    var best=limit;
    if (kind==3u) {
        let origin=(o-shape.a.xyz)/shape.b.xyz;
        let direction=d/shape.b.xyz;
        let candidates=roots(dot(direction,direction),dot(origin,direction),dot(origin,origin)-1.0);
        consider(i,o,d,candidates.x,&best);
        consider(i,o,d,candidates.y,&best);
    }
    if (kind==1u) {
        let core=max(shape.b.xyz-vec3f(shape.b.w),vec3f(0.0));
        let radius=shape.b.w;
        for (var axis=0u;axis<3u;axis++) {
            if (abs(d[axis])>1e-9) {
                for (var side=0u;side<2u;side++) {
                    let sign_side=select(-1.0,1.0,side==1u);
                    consider(i,o,d,(shape.a[axis]+sign_side*(core[axis]+radius)-o[axis])/d[axis],&best);
                }
            }
            let j=(axis+1u)%3u;
            let k=(axis+2u)%3u;
            for (var sj=0u;sj<2u;sj++) {
                for (var sk=0u;sk<2u;sk++) {
                    let js=select(-1.0,1.0,sj==1u);
                    let ks=select(-1.0,1.0,sk==1u);
                    let oj=o[j]-shape.a[j]-js*core[j];
                    let ok=o[k]-shape.a[k]-ks*core[k];
                    let candidates=roots(d[j]*d[j]+d[k]*d[k],oj*d[j]+ok*d[k],oj*oj+ok*ok-radius*radius);
                    for (var root=0u;root<2u;root++) {
                        let t=candidates[root];
                        if (abs(o[axis]+d[axis]*t-shape.a[axis])<=core[axis]+0.002) { consider(i,o,d,t,&best); }
                    }
                }
            }
        }
        for (var sx=0u;sx<2u;sx++) {
            for (var sy=0u;sy<2u;sy++) {
                for (var sz=0u;sz<2u;sz++) {
                    let corner=shape.a.xyz+vec3f(select(-1.0,1.0,sx==1u),select(-1.0,1.0,sy==1u),select(-1.0,1.0,sz==1u))*core;
                    let v=o-corner;
                    let candidates=roots(dot(d,d),dot(v,d),dot(v,v)-radius*radius);
                    consider(i,o,d,candidates.x,&best);
                    consider(i,o,d,candidates.y,&best);
                }
            }
        }
    }
    if (kind==4u) {
        let ba=shape.b.xyz-shape.a.xyz;
        let oa=o-shape.a.xyz;
        let baba=dot(ba,ba);
        let bard=dot(ba,d);
        let baoa=dot(ba,oa);
        let rdoa=dot(d,oa);
        let oaoa=dot(oa,oa);
        let radius=shape.c.x;
        let candidates=roots(baba*dot(d,d)-bard*bard,baba*rdoa-baoa*bard,baba*oaoa-baoa*baoa-radius*radius*baba);
        for (var root=0u;root<2u;root++) {
            let t=candidates[root];
            let y=baoa+t*bard;
            if (y>=0.0 && y<=baba) { consider(i,o,d,t,&best); }
        }
        for (var end=0u;end<2u;end++) {
            let center=select(shape.a.xyz,shape.b.xyz,end==1u);
            let v=o-center;
            let candidates=roots(dot(d,d),dot(v,d),dot(v,v)-radius*radius);
            consider(i,o,d,candidates.x,&best);
            consider(i,o,d,candidates.y,&best);
        }
    }
    if (kind==2u) {
        let ba=shape.b.xyz-shape.a.xyz;
        let len=length(ba);
        if (len>1e-8) {
            let axis=ba/len;
            let slope=(shape.c.x-shape.c.y)/len;
            if (abs(slope)<1.0) {
                let side=sqrt(1.0-slope*slope);
                let oa=o-shape.a.xyz;
                let y0=dot(oa,axis);
                let yd=dot(d,axis);
                let radial0=oa-axis*y0;
                let radiald=d-axis*yd;
                let e=shape.c.x-slope*y0;
                let candidates=roots(side*side*dot(radiald,radiald)-slope*slope*yd*yd,side*side*dot(radial0,radiald)+e*slope*yd,side*side*dot(radial0,radial0)-e*e);
                for (var root=0u;root<2u;root++) {
                    let t=candidates[root];
                    let y=y0+t*yd;
                    if (y>=slope*shape.c.x-0.002 && y<=len+slope*shape.c.y+0.002) { consider(i,o,d,t,&best); }
                }
            }
        }
        for (var end=0u;end<2u;end++) {
            let center=select(shape.a.xyz,shape.b.xyz,end==1u);
            let radius=select(shape.c.x,shape.c.y,end==1u);
            let v=o-center;
            let candidates=roots(dot(d,d),dot(v,d),dot(v,v)-radius*radius);
            consider(i,o,d,candidates.x,&best);
            consider(i,o,d,candidates.y,&best);
        }
    }
    return best;
}
fn union_hit(i: u32, o: vec3f, d: vec3f, limit: f32) -> f32 {
    var t=0.0;
    var last=shape_distance(i,o);
    for (var step=0u;step<384u;step++) {
        let previous=t;
        t+=max(abs(last),0.00005);
        if (t>=limit) { return limit; }
        let distance=shape_distance(i,o+d*t);
        if (abs(distance)<0.0001) { return t; }
        if ((last>0.0 && distance<0.0) || (last<0.0 && distance>0.0)) {
            var lo=previous;
            var hi=t;
            for (var refine=0u;refine<24u;refine++) {
                let mid=(lo+hi)*0.5;
                let value=shape_distance(i,o+d*mid);
                if (sign(value)==sign(last)) { lo=mid; } else { hi=mid; }
            }
            return (lo+hi)*0.5;
        }
        last=distance;
    }
    return limit;
}
fn scene(o: vec3f, d: vec3f, limit: f32, shadow_ray: bool) -> Hit {
    var hit=Hit(limit,vec3f(0.0),vec3f(0.0),0u,vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,0u);
    if (frame.counts.x>0u) {
        var stack: array<u32,64>;
        var top=1u;
        stack[0]=0u;
        while (top>0u) {
            top-=1u;
            let index=stack[top];
            let node=nodes[index];
            if (!box_hit(node,o,d,hit.t)) { continue; }
            if (node.count==0u) {
                if (top+2u<=64u) {
                    stack[top]=node.first;
                    stack[top+1u]=index+1u;
                    top+=2u;
                }
            } else {
                for (var j=0u;j<node.count;j++) { tri_hit(node.first+j,o,d,shadow_ray,&hit); }
            }
        }
    }
    for (var i=0u;i<frame.counts.y;i++) {
        if (shapes[i].d.x > 0.5) { continue; }
        var t=hit.t;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,o,d,hit.t); } else { t=primitive_hit(i,o,d,hit.t); }
        if (t<hit.t && t>0.0001) {
            let p=o+d*t;
            let e=0.0002;
            let n=normalize(vec3f(
                shape_distance(i,p+vec3f(e,0.0,0.0))-shape_distance(i,p-vec3f(e,0.0,0.0)),
                shape_distance(i,p+vec3f(0.0,e,0.0))-shape_distance(i,p-vec3f(0.0,e,0.0)),
                shape_distance(i,p+vec3f(0.0,0.0,e))-shape_distance(i,p-vec3f(0.0,0.0,e))));
            hit=Hit(t,n,n,u32(shapes[i].c.w),vec2f(0.0),vec3f(1.0,0.0,0.0),vec3f(0.0,1.0,0.0),0u,i+1u);
        }
    }
    return hit;
}
fn tri_blocks(i: u32, o: vec3f, d: vec3f, limit: f32) -> bool {
    let tri = triangles[i];
    if ((u32(tri.b.w)&2u)!=0u) { return false; }
    let e1 = tri.b.xyz-tri.a.xyz;
    let e2 = tri.c.xyz-tri.a.xyz;
    let p = cross(d,e2);
    let det = dot(e1,p);
    if (abs(det)<1e-9) { return false; }
    let inv = 1.0/det;
    let s = o-tri.a.xyz;
    let u = dot(s,p)*inv;
    if (u<0.0 || u>1.0) { return false; }
    let q = cross(s,e1);
    let v = dot(d,q)*inv;
    if (v<0.0 || u+v>1.0) { return false; }
    let t = dot(e2,q)*inv;
    if (t<=0.0001 || t>=limit) { return false; }
    let surface=instance_surfaces[i];
    let point=o+d*t;
    if ((any(surface.clip0.xyz != vec3f(0.0)) && dot(surface.clip0.xyz,point)>surface.clip0.w)
        || (any(surface.clip1.xyz != vec3f(0.0)) && dot(surface.clip1.xyz,point)>surface.clip1.w)) { return false; }
    return !cut_out(i,u,v);
}
fn occluded(o: vec3f, d: vec3f, limit: f32) -> bool {
    if (frame.counts.x>0u) {
        var stack: array<u32,64>;
        var top=1u;
        stack[0]=0u;
        while (top>0u) {
            top-=1u;
            let index=stack[top];
            let node=nodes[index];
            if (!box_hit(node,o,d,limit)) { continue; }
            if (node.count==0u) {
                if (top+2u<=64u) {
                    stack[top]=node.first;
                    stack[top+1u]=index+1u;
                    top+=2u;
                }
            } else {
                for (var j=0u;j<node.count;j++) {
                    if (tri_blocks(node.first+j,o,d,limit)) { return true; }
                }
            }
        }
    }
    for (var i=0u;i<frame.counts.y;i++) {
        if (shapes[i].d.x > 0.5) { continue; }
        var t=limit;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,o,d,limit); } else { t=primitive_hit(i,o,d,limit); }
        if (t<limit && t>0.0001) { return true; }
    }
    return false;
}
fn cast_tint(material: u32) -> vec3f {
    let transmission=materials[material].sheen_trans_ior_disp.y;
    if (transmission<=0.0) { return vec3f(0.0); }
    var passed=materials[material].base_roughness.xyz;
    let thickness=materials[material].thick_sub_film.x;
    let absorption=materials[material].tint_absorption.w;
    if (thickness>0.0 && absorption>0.0) { passed=beer(thickness,passed,absorption); }
    return clamp(passed*transmission,vec3f(0.0),vec3f(1.0));
}
fn tri_passes(i: u32, o: vec3f, d: vec3f, limit: f32) -> vec3f {
    if (!tri_blocks(i,o,d,limit)) { return vec3f(1.0); }
    return sqrt(cast_tint(u32(triangles[i].a.w)));
}
fn shadow_passed(o: vec3f, d: vec3f, limit: f32) -> vec3f {
    var passed=vec3f(1.0);
    if (frame.counts.x>0u) {
        var stack: array<u32,64>;
        var top=1u;
        stack[0]=0u;
        while (top>0u) {
            top-=1u;
            let index=stack[top];
            let node=nodes[index];
            if (!box_hit(node,o,d,limit)) { continue; }
            if (node.count==0u) {
                if (top+2u<=64u) {
                    stack[top]=node.first;
                    stack[top+1u]=index+1u;
                    top+=2u;
                }
            } else {
                for (var j=0u;j<node.count;j++) {
                    passed*=tri_passes(node.first+j,o,d,limit);
                    if (all(passed<=vec3f(0.0))) { return vec3f(0.0); }
                }
            }
        }
    }
    for (var i=0u;i<frame.counts.y;i++) {
        if (shapes[i].d.x > 0.5) { continue; }
        var t=limit;
        if (u32(shapes[i].a.w)==5u) { t=union_hit(i,o,d,limit); } else { t=primitive_hit(i,o,d,limit); }
        if (t<limit && t>0.0001) { passed*=cast_tint(u32(shapes[i].c.w)); }
    }
    return passed;
}
fn offset_axis(position: f32, normal: f32) -> f32 {
    if (abs(position) < 1.0/32.0) { return position + normal/65536.0; }
    let bits=bitcast<i32>(position);
    let shift=i32(normal*256.0);
    return bitcast<f32>(bits + select(shift, -shift, position < 0.0));
}
fn offset_ray(p: vec3f, n: vec3f) -> vec3f {
    return vec3f(offset_axis(p.x,n.x), offset_axis(p.y,n.y), offset_axis(p.z,n.z));
}
fn spawn_point(p: vec3f, dir: vec3f, origin: vec3f, t: f32) -> vec3f {
    let err=4e-7*(max(abs(origin.x),max(abs(origin.y),abs(origin.z)))+t);
    return offset_ray(p, dir)+dir*err;
}
fn basis(n: vec3f) -> mat3x3f {
    let t=normalize(cross(select(vec3f(0.0,1.0,0.0),vec3f(1.0,0.0,0.0),abs(n.y)>0.9),n));
    return mat3x3f(t,cross(n,t),n);
}
const ROUGHNESS_FLOOR: f32 = 0.05;
fn surface(material: Material) -> Shaded {
    return Shaded(material.base_roughness.xyz,material.base_roughness.w,
        material.metal_spec_coat.x,material.metal_spec_coat.y,material.metal_spec_coat.z,material.metal_spec_coat.w,
        material.sheen_trans_ior_disp.x,material.sheen_trans_ior_disp.y,material.sheen_trans_ior_disp.z,material.sheen_trans_ior_disp.w,
        material.thick_sub_film.x,material.thick_sub_film.y,material.tint_absorption.xyz,material.tint_absorption.w,
        material.thick_sub_film.z,material.thick_sub_film.w,material.emission_film.w,material.emission_film.xyz,
        material.age_b.w,vec3f(0.0,0.0,1.0));
}
fn detail_wobble(p: vec3f, scale: f32) -> vec3f {
    let q=p*scale;
    let e=0.35;
    return vec3f(value_noise3(q+vec3f(e,0.0,0.0),false)-value_noise3(q-vec3f(e,0.0,0.0),false),
        value_noise3(q+vec3f(0.0,e,0.0),false)-value_noise3(q-vec3f(0.0,e,0.0),false),
        value_noise3(q+vec3f(0.0,0.0,e),false)-value_noise3(q-vec3f(0.0,0.0,e),false))/(2.0*e);
}
fn detail_perturb(n: vec3f, g: vec3f) -> vec3f {
    return normalize(n-(g-n*dot(g,n)));
}
fn detail_layer(value: Shaded, normal_in: vec3f, layer: vec4f, pa: vec4f, pb: vec4f, pc: vec4f, pd: vec4f, position: vec3f) -> DetailResult {
    var shade=value;
    var base=value.base;
    var roughness=value.roughness;
    var coat_roughness=value.clearcoat_roughness;
    var n=normal_in;
    let kind=u32(round(layer.x));
    let amp=layer.z;
    let seed=bitcast<u32>(layer.w);
    let p=position+vec3f(f32(seed)*0.001,f32(seed)*0.001,0.0);
    if (kind==1u) {
        let fibre=value_noise3(p*vec3f(pa.x,pa.y,pa.x),false)*0.6+value_noise3(p*pa.z,false)*0.4;
        let mottle=value_noise3(p*pa.w,false);
        base*=1.0+((fibre-0.5)*pb.z+(mottle-0.5)*pb.w)*amp;
        n=detail_perturb(n,detail_wobble(p,pb.x)*(pc.x*amp)+detail_wobble(p,pb.y)*(pc.y*amp));
    } else if (kind==2u) {
        n=detail_perturb(n,detail_wobble(p,pa.x)*(pa.z*amp)+detail_wobble(p,pa.y)*(pa.w*amp));
    } else if (kind==3u && pa.x>0.0) {
        let plank=floor(p.z*pd.x);
        let tone=hash3_xor(vec3i(i32(plank),7,3));
        let across=fract(p.z*pd.x);
        let seam=smoothstep(0.0,0.006,across)+smoothstep(0.0,0.006,1.0-across)-1.0;
        let warp=value_noise3(vec3f(p.x*1.4,plank*3.7,p.z*6.0),false)*0.9+value_noise3(vec3f(p.x*6.0,plank,p.z*20.0),false)*0.25;
        let rings=fract(fma(fma(warp,pa.z,p.z),pa.y,value_noise3(vec3f(p.x*0.8,plank,0.0),false)*pc.y));
        let figure=smoothstep(0.55,0.95,rings)*0.55+value_noise3(vec3f(p.x*pa.w,p.z*pb.x,plank),false)*0.25;
        let factor=(0.78+tone*pb.y)*(1.08-figure*pb.z)*(pb.w+clamp(seam,0.0,1.0)*pd.y);
        base*=mix(1.0,factor,clamp(amp,0.0,1.0));
        roughness=mix(roughness,sqrt(min(roughness*roughness*(0.85+figure*pc.x),1.0)),clamp(amp,0.0,1.0));
    } else if (kind==4u) {
        let broad=value_noise3(p*pa.x,false)*0.6+value_noise3(p*pa.y,false)*0.4;
        let fine=value_noise3(p*pa.z,false)*0.6+value_noise3(p*pa.w,false)*0.4;
        base*=mix(1.0,1.0+(broad-0.5)*pb.z+(fine-0.5)*pb.w,clamp(amp,0.0,1.0));
        n=detail_perturb(n,detail_wobble(p,pb.x)*(pc.x*amp)+detail_wobble(p,pb.y)*(pc.y*amp));
    } else if (kind==5u) {
        let grime=value_noise3(p*pa.x,false)*0.6+value_noise3(p*pa.y,false)*0.4;
        let wear=smoothstep(0.35,0.75,grime);
        base*=mix(vec3f(1.0),mix(vec3f(pa.z,pa.w,pb.x),vec3f(pb.y,pb.z,pb.w),wear),clamp(amp,0.0,1.0));
        roughness=mix(roughness,sqrt(min(roughness*roughness*mix(pc.x,pc.y,wear),1.0)),clamp(amp,0.0,1.0));
    } else if (kind==15u) {
        base*=mix(1.0,1.0+(value_noise3(p*layer.y,false)-0.5)*2.0,clamp(amp,0.0,1.0));
    } else if (kind==17u) {
        let line=1.0-smoothstep(0.0,0.045,abs(value_noise3(vec3f(p.x*pd.x+f32(seed),p.y*pa.x,p.z*pd.x),false)-0.5));
        let smudge=smoothstep(0.52,0.78,value_noise3(p*pa.y,false)*0.65+value_noise3(p*pa.z,false)*0.35);
        let cell=vec3i(floor(p*pa.w));
        let speck=select(0.0,1.0,hash3_xor(vec3i(cell.x,cell.y,cell.z^i32(seed)))>0.9985);
        let breakup=clamp(smudge*0.55+line*0.35+speck,0.0,1.0)*min(amp,1.0);
        if (value.clearcoat>0.0) {
            coat_roughness=sqrt(mix(coat_roughness*coat_roughness,min(coat_roughness*coat_roughness*4.0+0.02,0.35),breakup));
        } else {
            roughness=sqrt(mix(roughness*roughness,min(roughness*roughness*4.0+0.02,0.35),breakup));
        }
        base=mix(base,base*0.9+vec3f(0.06),speck*min(amp,1.0));
    } else if (kind==18u) {
        n=detail_perturb(n,detail_wobble(p,pa.x)*amp+detail_wobble(p,pa.y)*(amp*pa.w)+detail_wobble(p,pa.z)*(amp*pb.x));
    }
    shade.base=base;
    shade.roughness=roughness;
    shade.clearcoat_roughness=coat_roughness;
    return DetailResult(shade,n);
}
fn leaf_hash(p: vec2f) -> f32 {
    let q=fract(p*vec2f(123.34,456.21));
    return fract((q.x+45.32)*(q.y+45.32)*34.23+q.x*13.7);
}
fn leaf_noise(p: vec2f) -> f32 {
    let i=floor(p);
    let f=fract(p);
    let u=f*f*(3.0-2.0*f);
    return mix(mix(leaf_hash(i),leaf_hash(i+vec2f(1.0,0.0)),u.x),
        mix(leaf_hash(i+vec2f(0.0,1.0)),leaf_hash(i+vec2f(1.0,1.0)),u.x),u.y);
}
fn leaf_fbm(p: vec2f) -> f32 {
    return leaf_noise(p)*0.55+leaf_noise(p*2.13+3.1)*0.28+leaf_noise(p*4.37+7.7)*0.17;
}
fn canopy(p: vec3f) -> f32 {
    let sun=normalize(frame.sun_dir.xyz);
    let e1=normalize(cross(sun,vec3f(0.0,1.0,0.0)));
    let e2=cross(e1,sun);
    let q=vec2f(dot(p-effects().metal_spec_coat.xyz,e1),dot(p-effects().metal_spec_coat.xyz,e2));
    let t=effects().base_roughness.x;
    let gust=sin(t*0.21)*0.6+sin(t*0.37+1.3)*0.4;
    let sway=vec2f(sin(t*0.9+q.y*9.0)*0.005+gust*0.011,cos(t*0.7+q.x*7.0)*0.004+gust*0.004);
    let shifted=q+sway;
    let branch=smoothstep(-0.55,0.15,shifted.x*0.8+shifted.y*0.6);
    let clusters=smoothstep(0.3,0.62,leaf_fbm(shifted*3.4+vec2f(5.0,2.0)));
    let leaves=smoothstep(0.36,0.56,leaf_fbm(shifted*17.0+vec2f(sin(t*1.3)*0.08,cos(t*1.1)*0.08)));
    return mix(1.0,1.0-leaves*clusters*branch*0.88,effects().base_roughness.y);
}
fn steam_fbm(p: vec3f) -> f32 {
    var sum=0.0;
    var amp=0.55;
    var q=p;
    for (var k=0;k<4;k++) {
        sum+=value_noise3(q,false)*amp;
        q=q*2.07+vec3f(1.7,9.2,3.1);
        amp*=0.5;
    }
    return sum;
}
fn steam_density(p: vec3f) -> f32 {
    let t=effects().base_roughness.w;
    let source=effects().sheen_trans_ior_disp.xyz;
    let rise=p.y-source.y;
    if (rise<0.0 || rise>STEAM_FADE_OUT_END) { return 0.0; }
    let drift=vec3f(STEAM_DRIFT_X_QUAD*rise*rise+STEAM_DRIFT_X_LIN*rise,0.0,STEAM_DRIFT_Z_QUAD*rise*rise+STEAM_DRIFT_Z_LIN*rise);
    let sway=vec3f(sin(rise*STEAM_SWAY_X_WAVE-t*STEAM_SWAY_X_RATE)*STEAM_SWAY_X_AMP*(STEAM_SWAY_X_BASE+rise*STEAM_SWAY_X_GROWTH),0.0,
        cos(rise*STEAM_SWAY_Z_WAVE-t*STEAM_SWAY_Z_RATE)*STEAM_SWAY_Z_AMP*(STEAM_SWAY_Z_BASE+rise*STEAM_SWAY_Z_GROWTH));
    let axis=source+vec3f(0.0,rise,0.0)+drift+sway;
    let width=effects().sheen_trans_ior_disp.w*(STEAM_SPREAD_BASE+rise*STEAM_SPREAD_GROWTH);
    let r=length((p-axis).xz);
    let core=exp(-r*r/(width*width));
    let fade=smoothstep(0.0,STEAM_FADE_IN,rise)*(1.0-smoothstep(STEAM_FADE_OUT_START,STEAM_FADE_OUT_END,rise));
    if (core*fade<0.002) { return 0.0; }
    let q=vec3f(p.x*STEAM_GRAIN_H,(p.y-t*STEAM_LIFT_RATE)*STEAM_GRAIN_V,p.z*STEAM_GRAIN_H);
    let warp=vec3f(steam_fbm(q*0.5+vec3f(t*STEAM_WARP_RATE,0.0,0.0)),0.0,
        steam_fbm(q*0.5+vec3f(3.1,0.0,t*STEAM_WARP_RATE)))*STEAM_WARP_AMP;
    let turbulence=smoothstep(STEAM_THRESHOLD_LOW,STEAM_THRESHOLD_HIGH,steam_fbm(q+warp+vec3f(0.0,-t*STEAM_CHURN,0.0)));
    return core*turbulence*fade*effects().thick_sub_film.w;
}
fn steam_composite(origin: vec3f, direction: vec3f, surface_depth: f32, color: vec3f) -> vec3f {
    if (effects().thick_sub_film.w<=0.0) { return color; }
    let inv=1.0/select(direction,sign(direction)*vec3f(1e-8),abs(direction)<vec3f(1e-8));
    let t0=(effects().thick_sub_film.xyz-origin)*inv;
    let t1=(effects().tint_absorption.xyz-origin)*inv;
    let enter=max(max(min(t0.x,t1.x),min(t0.y,t1.y)),max(min(t0.z,t1.z),0.0));
    let leave=min(min(max(t0.x,t1.x),max(t0.y,t1.y)),min(max(t0.z,t1.z),surface_depth));
    if (enter>=leave) { return color; }
    let dt=(leave-enter)/20.0;
    var transmittance=1.0;
    var scattered=vec3f(0.0);
    let sun=normalize(frame.sun_dir.xyz);
    let g=effects().metal_spec_coat.w;
    let phase=(1.0-g*g)/pow(1.0+g*g-2.0*g*dot(direction,sun),1.5);
    for (var k=0u;k<20u;k++) {
        let p=origin+direction*(enter+(f32(k)+0.5)*dt);
        let density=steam_density(p);
        if (density>1e-4) {
            let extinction=exp(-density*90.0*dt);
            let open=select(1.0,0.0,scene(spawn_point(p,sun,p,0.0),sun,1e20,true).t<1e19);
            let lit=frame.sun_color.xyz*frame.sun_color.w*open*phase;
            scattered+=transmittance*(lit+vec3f(effects().tint_absorption.w))*(1.0-extinction)*0.85;
            transmittance*=extinction;
        }
    }
    return scattered+transmittance*color;
}
fn texels_at(offset: u32, width: u32, height: u32, uv: vec2f) -> vec4f {
    let size=vec2f(f32(width),f32(height));
    let p=clamp(uv*size-vec2f(0.5),vec2f(0.0),size-vec2f(1.0));
    let lo=vec2u(floor(p));
    let hi=min(lo+vec2u(1u),vec2u(width-1u,height-1u));
    let f=fract(p);
    let a=content_texels[offset+lo.y*width+lo.x];
    let b=content_texels[offset+lo.y*width+hi.x];
    let c=content_texels[offset+hi.y*width+lo.x];
    let d=content_texels[offset+hi.y*width+hi.x];
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn atlas_texel(offset: u32, width: u32, x: u32, y: u32) -> vec3f {
    let at=y*width+x;
    let packed=u32(content_texels[offset+at/4u][at&3u]);
    return vec3f(f32(packed&255u),f32((packed>>8u)&255u),f32(packed>>16u))/255.0;
}
fn atlas_at(offset: u32, width: u32, height: u32, uv: vec2f) -> vec3f {
    let p=uv*vec2f(f32(width),f32(height))-vec2f(0.5);
    let start=floor(p);
    let f=p-start;
    let lo=vec2u(clamp(start,vec2f(0.0),vec2f(f32(width-1u),f32(height-1u))));
    let hi=vec2u(clamp(start+vec2f(1.0),vec2f(0.0),vec2f(f32(width-1u),f32(height-1u))));
    let a=atlas_texel(offset,width,lo.x,lo.y);
    let b=atlas_texel(offset,width,hi.x,lo.y);
    let c=atlas_texel(offset,width,lo.x,hi.y);
    let d=atlas_texel(offset,width,hi.x,hi.y);
    return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
fn text_at(offset: u32, uv: vec2f) -> vec4f {
    let grid=content_texels[offset];
    let shape=vec4u(content_texels[offset+1u]);
    let atlas=vec4u(content_texels[offset+2u]);
    let picture=vec4u(content_texels[offset+3u]);
    var color=vec4f(0.0);
    if (picture.x>0u) { color=texels_at(offset+picture.z,picture.x,picture.y,uv); }
    let cell=floor((uv-grid.xy)*grid.zw);
    if (cell.x<0.0 || cell.y<0.0 || cell.x>=f32(shape.x) || cell.y>=f32(shape.y)) { return color; }
    let index=u32(cell.y)*shape.x+u32(cell.x);
    let pair=content_texels[offset+4u+index/2u];
    let entry=vec2u(select(pair.xy,pair.zw,(index&1u)==1u));
    let size=vec2f(f32(atlas.x),f32(atlas.y));
    let half=vec2f(0.5)/size;
    for (var k=0u;k<entry.y;k++) {
        let at=entry.x+k;
        let glyph=offset+shape.z+u32(content_texels[offset+shape.w+at/4u][at&3u])*5u;
        let m=content_texels[glyph];
        let t=content_texels[glyph+1u];
        let clip=content_texels[glyph+2u];
        let box=content_texels[glyph+3u];
        let ink=content_texels[glyph+4u];
        let f=vec2f(m.x*uv.x+m.y*uv.y+t.x,m.z*uv.x+m.w*uv.y+t.y);
        if (any(f<max(clip.xy,vec2f(0.0))) || any(f>min(clip.zw,vec2f(1.0)))) { continue; }
        let low=min(box.xy+half,box.zw);
        let high=max(box.zw-half,low);
        let sampled=atlas_at(offset+atlas.z,atlas.x,atlas.y,clamp(box.xy+(box.zw-box.xy)*f,low,high));
        var coverage=sampled.x;
        if (atlas.w==3u) {
            let median=max(min(sampled.x,sampled.y),min(max(sampled.x,sampled.y),sampled.z));
            coverage=select(0.0,1.0,median>=0.5);
        }
        let opacity=coverage*ink.a;
        color=vec4f(ink.rgb*opacity+color.rgb*(1.0-opacity),opacity+color.a*(1.0-opacity));
    }
    return color;
}
fn content_at(slot: i32, uv: vec2f) -> vec4f {
    if (slot < 0) { return vec4f(0.0); }
    let info=content_info[u32(slot)];
    if (info.width==0u || info.height==0u) { return vec4f(0.0); }
    if (info.kind==1u) { return text_at(info.offset,uv); }
    return texels_at(info.offset,info.width,info.height,uv);
}
const CF_WEIGHTS = array<vec3f, 81>(
    vec3f(5.039987e-05, 0.0, 0.00035778867),
    vec3f(6.775547e-05, 0.0, 0.0005768482),
    vec3f(0.00013374595, 0.0, 0.001105031),
    vec3f(0.000244155, -8.0429134e-05, 0.001990094),
    vec3f(0.0004620315, -0.00031065758, 0.0037327078),
    vec3f(0.00075178174, -0.00079914776, 0.0060585286),
    vec3f(0.0013901147, -0.0016784245, 0.01140122),
    vec3f(0.0024506692, -0.0030452313, 0.020409657),
    vec3f(0.0041841697, -0.00489408, 0.035484146),
    vec3f(0.006491386, -0.0070713423, 0.05709876),
    vec3f(0.008225769, -0.009268977, 0.07611217),
    vec3f(0.00893323, -0.0110506825, 0.08910742),
    vec3f(0.008651618, -0.011844616, 0.09585401),
    vec3f(0.0075308583, -0.0111710355, 0.09771433),
    vec3f(0.00574333, -0.009713182, 0.0970225),
    vec3f(0.0034815131, -0.0075853216, 0.09534322),
    vec3f(0.00069844024, -0.0048573813, 0.091060266),
    vec3f(-0.0036886954, -0.0017018986, 0.083120264),
    vec3f(-0.0063273897, 0.0017186198, 0.06969027),
    vec3f(-0.008724144, 0.005763426, 0.05593676),
    vec3f(-0.011347991, 0.009961927, 0.04306675),
    vec3f(-0.014388451, 0.014161423, 0.031928584),
    vec3f(-0.017747117, 0.018677758, 0.023228362),
    vec3f(-0.021200482, 0.023959707, 0.016556388),
    vec3f(-0.024560854, 0.030223139, 0.011426527),
    vec3f(-0.027776832, 0.03802389, 0.00728438),
    vec3f(-0.030845057, 0.046441782, 0.0033540253),
    vec3f(-0.03346058, 0.055137716, 0.0),
    vec3f(-0.03508027, 0.06285677, -0.0025753065),
    vec3f(-0.035013195, 0.068301246, -0.004734569),
    vec3f(-0.032630157, 0.071965545, -0.006312869),
    vec3f(-0.027983842, 0.07395785, -0.00737929),
    vec3f(-0.02177047, 0.07446333, -0.008106243),
    vec3f(-0.0140442625, 0.07356939, -0.008540686),
    vec3f(-0.0049150633, 0.07139566, -0.008724192),
    vec3f(0.0046487506, 0.06808836, -0.008696091),
    vec3f(0.0153741175, 0.063680954, -0.008493348),
    vec3f(0.026958833, 0.058147695, -0.008150433),
    vec3f(0.03911687, 0.051680956, -0.007698217),
    vec3f(0.05145111, 0.04444711, -0.0071432022),
    vec3f(0.063475594, 0.036714625, -0.006503886),
    vec3f(0.07455033, 0.028763412, -0.005807879),
    vec3f(0.08411886, 0.020992132, -0.0050818375),
    vec3f(0.091669224, 0.0137893725, -0.00435086),
    vec3f(0.09618923, 0.007610871, -0.0036377995),
    vec3f(0.097940184, 0.002461353, -0.0029763517),
    vec3f(0.09633994, -0.0015017615, -0.0023928043),
    vec3f(0.0919427, -0.004451467, -0.0018944021),
    vec3f(0.08495084, -0.0062283436, -0.0014817494),
    vec3f(0.07555293, -0.0070042815, -0.0011499332),
    vec3f(0.06515909, -0.006994329, -0.00089005305),
    vec3f(0.05535661, -0.0064279693, -0.00069090264),
    vec3f(0.046014983, -0.005523557, -0.0005405573),
    vec3f(0.037232462, -0.004469, -0.00042767118),
    vec3f(0.029350668, -0.0034102271, -0.0003423706),
    vec3f(0.022698376, -0.0024474342, -0.00027671963),
    vec3f(0.017146055, -0.001637843, -0.00022480598),
    vec3f(0.012616245, -0.0010029421, -0.00018254654),
    vec3f(0.00910757, -0.0005379737, -0.00014732922),
    vec3f(0.0066325837, -0.00022167167, -0.00011759913),
    vec3f(0.004884865, -2.4797864e-05, -9.247229e-05),
    vec3f(0.0034370946, 0.0, -7.142631e-05),
    vec3f(0.0023721277, 0.0, -5.40864e-05),
    vec3f(0.0016515272, 0.0, -4.010171e-05),
    vec3f(0.0011923393, 0.0, -2.9092696e-05),
    vec3f(0.00084797293, 0.0, -2.064557e-05),
    vec3f(0.000605779, 0.0, -1.4331459e-05),
    vec3f(0.0004273203, 0.0, -9.733269e-06),
    vec3f(0.00030592916, 0.0, -6.469539e-06),
    vec3f(0.00021032631, 0.0, -4.210301e-06),
    vec3f(0.00014668911, 0.0, -2.683982e-06),
    vec3f(0.0001021725, 0.0, -1.6768181e-06),
    vec3f(7.628618e-05, 0.0, -1.027186e-06),
    vec3f(5.108625e-05, 0.0, -6.1728224e-07),
    vec3f(3.186762e-05, 0.0, -3.6408065e-07),
    vec3f(1.9218627e-05, 0.0, -2.1085688e-07),
    vec3f(1.9218627e-05, 0.0, -1.1996055e-07),
    vec3f(1.2648995e-05, 0.0, 2.5954927e-07),
    vec3f(1.2648995e-05, 0.0, 2.5954927e-07),
    vec3f(1.2648995e-05, 0.0, 2.5954927e-07),
    vec3f(-5.4965028e-08, 0.0, -1.0592136e-08),
);
fn cf_fresnel(cos_i: f32, na: f32, nb: f32) -> vec3f {
    let sin2=(na/nb)*(na/nb)*(1.0-cos_i*cos_i);
    if (sin2>=1.0) { return vec3f(1.0,1.0,0.0); }
    let cos_t=sqrt(1.0-sin2);
    let rs=(na*cos_i-nb*cos_t)/(na*cos_i+nb*cos_t);
    let rp=(nb*cos_i-na*cos_t)/(nb*cos_i+na*cos_t);
    return vec3f(rs,rp,cos_t);
}
fn cf_film(cosine: f32, thickness: f32, n1: f32, nf: f32, n3: f32) -> vec3f {
    let first=cf_fresnel(clamp(cosine,0.0,1.0),n1,nf);
    if (first.z<=0.0) { return vec3f(1.0); }
    let second=cf_fresnel(first.z,nf,n3);
    if (second.z<=0.0) { return vec3f(1.0); }
    let r12=first.xy;
    let r23=second.xy;
    let a=r12*r12+r23*r23;
    let b=2.0*r12*r23;
    let e=1.0+r12*r12*r23*r23;
    let path=4.0*PI*nf*thickness*first.z;
    var weights=CF_WEIGHTS;
    var total=vec3f(0.0);
    for (var k=0u;k<81u;k++) {
        let c=cos(path/(380.0+5.0*f32(k)));
        let reflect=0.5*((a.x+b.x*c)/(e.x+b.x*c)+(a.y+b.y*c)/(e.y+b.y*c));
        total+=weights[k]*reflect;
    }
    return clamp(total,vec3f(0.0),vec3f(1.0));
}
fn film_active(s: Shaded) -> bool {
    return s.thin_film>0.0 && s.thin_film_amount>0.0;
}
fn cf_surface(cosine: f32, thickness: f32, film_ior: f32, ior: f32, entering: bool) -> vec3f {
    let nf=max(film_ior,1.01);
    if (entering) { return cf_film(cosine,thickness,1.0,nf,ior); }
    return cf_film(cosine,thickness,1.0,nf/ior,1.0/ior);
}
fn transmissive_fresnel(s: Shaded, cosine: f32, ratio: f32, entering: bool, ior: f32) -> vec3f {
    let plain=vec3f(fresnel_dielectric(cosine,ratio));
    if (!film_active(s)) { return plain; }
    let filmed=cf_surface(cosine,s.thin_film,s.thin_film_ior,ior,entering);
    return mix(plain,filmed,clamp(s.thin_film_amount,0.0,1.0));
}
const IOR_SPLIT: f32 = 1e-4;
fn channel_mask(channel: u32) -> vec3f {
    return vec3f(select(0.0,1.0,channel==0u),select(0.0,1.0,channel==1u),select(0.0,1.0,channel==2u));
}
fn channel_ior(s: Shaded, channel: u32) -> f32 {
    if (channel>2u) { return max(s.ior,1.01); }
    return max(dispersed_ior(s.ior,s.dispersion)[channel],1.01);
}
fn glossy_share(s: Shaded) -> f32 {
    return 1.0-clamp(s.transmission,0.0,1.0)*(1.0-clamp(s.metalness,0.0,1.0));
}
fn sample_bsdf(s: Shaded, wo: vec3f, entering: bool, carried: u32, state: ptr<function,u32>) -> LightSample {
    let transmission=clamp(s.transmission,0.0,1.0)*(1.0-clamp(s.metalness,0.0,1.0));
    if (random(state)<transmission) {
        let iors=dispersed_ior(s.ior,s.dispersion);
        var channel=carried;
        var weight=vec3f(1.0);
        if (channel>2u && max(abs(iors.x-iors.y),abs(iors.z-iors.y))>IOR_SPLIT) {
            channel=min(u32(random(state)*3.0),2u);
            weight=channel_mask(channel)*3.0;
        }
        let ior=channel_ior(s,channel);
        let ratio=select(1.0/ior,ior,entering);
        let live=select(channel_mask(channel),vec3f(1.0),channel>2u);
        let fresnel=transmissive_fresnel(s,max(wo.z,0.0),ratio,entering,ior)*live;
        let chance=dot(fresnel,vec3f(1.0))/dot(live,vec3f(1.0));
        let incident=-wo;
        let mirrored=reflect(incident,vec3f(0.0,0.0,1.0));
        if (random(state)<chance) {
            return LightSample(mirrored,-1.0,channel,weight*fresnel/chance,1u);
        }
        let refracted=refract_dir(incident,vec3f(0.0,0.0,1.0),1.0/ratio);
        if (refracted.w>0.0) { return LightSample(normalize(refracted.xyz),-2.0,channel,weight*(live-fresnel)/(1.0-chance),2u); }
        return LightSample(mirrored,-1.0,channel,weight*live,1u);
    }
    let coat_weight=clamp(s.clearcoat*(0.12+0.88*(0.04+0.96*pow(1.0-max(wo.z,0.0),5.0))),0.0,0.9);
    let spec_weight=(1.0-coat_weight)*clamp(s.metalness+s.specular,0.0,0.9);
    let pick=random(state);
    if (pick<coat_weight+spec_weight) {
        let coat=pick<coat_weight;
        let alpha=alpha_rough(select(s.roughness,s.clearcoat_roughness,coat));
        let u1=random(state);
        let u2=random(state);
        let phi=2.0*PI*u1;
        let cos_theta=sqrt((1.0-u2)/(1.0+(alpha*alpha-1.0)*u2));
        let h=vec3f(cos(phi)*sqrt(max(0.0,1.0-cos_theta*cos_theta)),sin(phi)*sqrt(max(0.0,1.0-cos_theta*cos_theta)),cos_theta);
        let wi=reflect(-wo,h);
        return LightSample(wi,bsdf_pdf(s,wo,wi),3u,vec3f(1.0),1u);
    }
    let u1=random(state);
    let u2=random(state);
    let phi=2.0*PI*u1;
    let r=sqrt(u2);
    let wi=vec3f(cos(phi)*r,sin(phi)*r,sqrt(1.0-u2));
    return LightSample(wi,bsdf_pdf(s,wo,wi),3u,vec3f(1.0),0u);
}
fn bsdf_pdf(s: Shaded, wo: vec3f, wi: vec3f) -> f32 {
    if (wi.z<=0.0) { return 0.0; }
    let trans=clamp(s.transmission,0.0,1.0)*(1.0-clamp(s.metalness,0.0,1.0));
    let coat=clamp(s.clearcoat*(0.12+0.88*(0.04+0.96*pow(1.0-max(wo.z,0.0),5.0))),0.0,0.9);
    let spec=(1.0-coat)*clamp(s.metalness+s.specular,0.0,0.9);
    let h=normalize(wo+wi);
    let a=alpha_rough(s.roughness);
    let ca=alpha_rough(s.clearcoat_roughness);
    let denom=max(4.0*abs(dot(wi,h)),1e-8);
    let base_pdf=ggx_d(max(h.z,0.0),a*a)*max(h.z,0.0)/denom;
    let coat_pdf=ggx_d(max(h.z,0.0),ca*ca)*max(h.z,0.0)/denom;
    return (1.0-trans)*(coat*coat_pdf+spec*base_pdf+(1.0-coat-spec)*wi.z/PI);
}
fn trace_lights_start() {}
fn emitter_weight(hit: Hit, p: vec3f, previous_point: vec3f, previous_pdf: f32) -> f32 {
    return 1.0;
}
fn light_hits(o: vec3f, d: vec3f, limit: f32, previous_point: vec3f, previous_pdf: f32) -> vec3f {
    return vec3f(0.0);
}
fn light_slots() -> u32 {
    return 0u;
}
fn sampled_emitter(hit: Hit) -> bool {
    return false;
}
fn direct_light(slot: u32, transform: mat3x3f, p: vec3f, origin: vec3f, geometric: vec3f, state: ptr<function,u32>) -> Candidate {
    return Candidate(vec3f(0.0,0.0,1.0),vec3f(0.0),0.0,0.0,vec3f(0.0,0.0,1.0),-1.0,0u);
}
fn trace_lights_end() {}
fn catcher_visible(origin: vec3f, direction: vec3f, limit: f32) -> f32 {
    if (TRANSMISSIVE_SHADOWS) { return dot(shadow_passed(origin,direction,limit),vec3f(0.2126,0.7152,0.0722)); }
    return select(1.0,0.0,occluded(origin,direction,limit));
}
fn catcher_shadow(p: vec3f, n: vec3f, o: vec3f, t: f32, state: ptr<function,u32>) -> f32 {
    let transform=basis(n);
    let origin=spawn_point(p,n,o,t);
    var open=0.0;
    var seen=0.0;
    for (var k=0u;k<2u+light_slots();k++) {
        var direction=vec3f(0.0,0.0,1.0);
        var strength=0.0;
        var limit=1e19;
        var shape=0u;
        if (k==0u) {
            if (frame.sun_color.w<=0.0) { continue; }
            direction=normalize(frame.sun_dir.xyz);
            if (frame.sun_dir.w<1.0) {
                let height=1.0-random(state)*(1.0-frame.sun_dir.w);
                let radius=sqrt(max(0.0,1.0-height*height));
                let phi=2.0*PI*random(state);
                direction=normalize(basis(direction)*vec3f(radius*cos(phi),radius*sin(phi),height));
            }
            strength=dot(frame.sun_color.xyz,vec3f(0.2126,0.7152,0.0722))*frame.sun_color.w*max(dot(direction,n),0.0);
        } else if (k==1u) {
            if (sky_cdf.width==0u) { continue; }
            for (var j=0u;j<4u;j++) {
                let light=sky_sample(vec2f(random(state),random(state)));
                if (!(light.pdf>0.0)) { continue; }
                let part=dot(sky_surface_radiance(light.direction),vec3f(0.2126,0.7152,0.0722))*max(dot(light.direction,n),0.0)/light.pdf*0.25;
                if (!(part>0.0)) { continue; }
                open+=part;
                seen+=part*catcher_visible(origin,light.direction,1e19);
            }
            continue;
        } else {
            let picked=direct_light(k-2u,transform,p,origin,n,state);
            if (picked.limit<0.0) { continue; }
            direction=picked.direction;
            limit=picked.limit;
            shape=picked.shape;
            strength=dot(picked.scale,vec3f(0.2126,0.7152,0.0722))*max(picked.wi.z,0.0)/select(1.0,picked.pdf,picked.pdf>0.0);
        }
        if (!(strength>0.0)) { continue; }
        if (shape>0u) {
            let i=shape-1u;
            var hit_t=1e20;
            if (u32(shapes[i].a.w)==5u) { hit_t=union_hit(i,origin,direction,1e20); } else { hit_t=primitive_hit(i,origin,direction,1e20); }
            if (hit_t<=0.0001 || hit_t>=1e19) { continue; }
            limit=hit_t*0.9999-0.0002;
        }
        open+=strength;
        if (limit>0.0) { seen+=strength*catcher_visible(origin,direction,limit); } else { seen+=strength; }
    }
    if (open<=0.0) { return 0.0; }
    return clamp(1.0-seen/open,0.0,1.0);
}
const INDIRECT_LUMA: vec3f = vec3f(0.2126,0.7152,0.0722);
fn indirect_capped(value: vec3f, deep: bool) -> vec3f {
    if (!(CLAMP_INDIRECT>0.0) || !deep) { return value; }
    let luma=dot(value,INDIRECT_LUMA);
    if (!(luma>CLAMP_INDIRECT)) { return value; }
    return value*(CLAMP_INDIRECT/luma);
}
fn radiance(o0: vec3f, d0: vec3f, state: ptr<function,u32>, first_albedo: ptr<function,vec4f>, first_normal: ptr<function,vec4f>) -> vec3f {
    var o=o0;
    var d=d0;
    var throughput=vec3f(1.0);
    var result=vec3f(0.0);
    var previous_pdf=0.0;
    var carried=3u;
    var lobes=vec3u(0u);
    var last=false;
    var lit_vertex=false;
    var through=false;
    var diffused=false;
    for (var bounce=0u;bounce<=TOTAL_BOUNCES+1u;bounce++) {
        let hit=scene(o,d,1e20,false);
        let seen=TRANSMISSIVE_SHADOWS && through;
        if (!seen) { result+=indirect_capped(throughput*light_hits(o,d,hit.t,spawn_point(o,-d,o,0.0),previous_pdf),bounce>1u); }
        if (hit.t>=1e19) {
            if (bounce==0u && frame.forward.w>0.0) { break; }
            if (seen && sky_cdf.width>0u) { break; }
            let pdf=sky_pdf(d);
            let weight=select(1.0,previous_pdf/(previous_pdf+pdf),bounce>0u && previous_pdf>0.0);
            let environment=select(sky_radiance(d),sky_surface_radiance(d),bounce>0u);
            result+=indirect_capped(throughput*environment*weight,bounce>1u);
            break;
        }
        let p=o+d*hit.t;
        var n=select(hit.n,-hit.n,dot(hit.n,d)>0.0);
        if (bounce==0u && frame.forward.w>1.5 && hit.material==u32(frame.forward.w)-2u) {
            *first_albedo=vec4f(0.0,0.0,0.0,catcher_shadow(p,n,o,hit.t,state));
            *first_normal=vec4f(n,1.0);
            break;
        }
        let m=materials[hit.material];
        var s=surface(m);
        let layers=array<vec4f,4>(m.layer0,m.layer1,m.layer2,m.layer3);
        for (var i=0u;i<4u;i++) {
            if (layers[i].x>0.0 && layers[i].z!=0.0) {
                let detailed=detail_layer(s,n,layers[i],m.params[i*4u],m.params[i*4u+1u],m.params[i*4u+2u],m.params[i*4u+3u],p);
                s=detailed.shade;
                n=detailed.normal;
            }
        }
        let geometric=select(hit.geometric,-hit.geometric,dot(hit.geometric,d)>0.0);
        if (dot(n,geometric)<0.05) { n=normalize(n+geometric*0.1); }
        var layer=content_layers[hit.material];
        var shown=true;
        if (hit.shape==0u) {
            layer=hit_layer(hit.surface,hit.material);
            let face=(u32(triangles[hit.surface].b.w)>>3u)&3u;
            let front=dot(hit.geometric,d)<0.0;
            shown=face==0u || (face==1u && front) || (face==2u && !front);
        }
        let slot=i32(round(layer.layer.x));
        if (slot>=0 && shown) {
            let surface_data=instance_surfaces[hit.surface];
            let at=content_uv(hit.uv,surface_data.transform);
            if (content_inside(at,surface_data.crop)) {
                let texel=content_at(slot,at);
                let blend=u32(round(layer.layer.y));
                s=content_composite(s,blend,texel,layer.layer.z,layer.extra.x);
                if (layer.layer.w!=0.0) {
                    let info=content_info[u32(slot)];
                    if (info.width>0u && info.height>0u) {
                        let tangent=normalize(hit.tangent-n*dot(hit.tangent,n));
                        let bitangent=normalize(hit.bitangent-n*dot(hit.bitangent,n));
                        let step=vec2f(1.5/f32(info.width),1.5/f32(info.height));
                        let gx=content_at(slot,at+vec2f(step.x,0.0)).a-content_at(slot,at-vec2f(step.x,0.0)).a;
                        let gy=content_at(slot,at+vec2f(0.0,step.y)).a-content_at(slot,at-vec2f(0.0,step.y)).a;
                        n=content_emboss(n,tangent,bitangent,vec2f(gx,gy),layer.layer.w);
                        let geometric=select(hit.geometric,-hit.geometric,dot(hit.geometric,d)>0.0);
                        if (dot(n,geometric)<0.05) { n=normalize(n+geometric*0.1); }
                    }
                }
            }
        }
        s.roughness=max(s.roughness,ROUGHNESS_FLOOR);
        s.clearcoat_roughness=max(s.clearcoat_roughness,ROUGHNESS_FLOOR);
        if (FILTER_GLOSSY>0.0 && diffused) {
            s.roughness=max(s.roughness,FILTER_GLOSSY);
            s.clearcoat_roughness=max(s.clearcoat_roughness,FILTER_GLOSSY);
        }
        if (bounce==0u) {
            *first_albedo=vec4f(s.base,1.0);
            *first_normal=vec4f(n,1.0);
        }
        s.emission*=emitter_weight(hit,p,spawn_point(o,-d,o,0.0),previous_pdf);
        if (seen && sampled_emitter(hit)) { s.emission=vec3f(0.0); }
        result+=indirect_capped(throughput*s.emission,bounce>1u);
        if (last) { break; }
        let transform=basis(n);
        let wo=transpose(transform)*(-d);
        let origin=spawn_point(p,n,o,hit.t);
        let entering=dot(hit.n,d)<0.0;
        for (var k=0u;k<2u+light_slots();k++) {
            var local=vec3f(0.0,0.0,1.0);
            var direction=vec3f(0.0,0.0,1.0);
            var strength=vec3f(0.0);
            var limit=1e19;
            var shape=0u;
            var pdf=0.0;
            var lit=s;
            if (k==0u) {
                if (frame.sun_color.w<=0.0) { continue; }
                direction=normalize(frame.sun_dir.xyz);
                if (frame.sun_dir.w<1.0) {
                    let height=1.0-random(state)*(1.0-frame.sun_dir.w);
                    let radius=sqrt(max(0.0,1.0-height*height));
                    let phi=2.0*PI*random(state);
                    let orient=basis(direction);
                    direction=normalize(orient*vec3f(radius*cos(phi),radius*sin(phi),height));
                }
                local=transpose(transform)*direction;
                if (local.z<=0.0) { continue; }
                var shade=1.0;
                if (bounce==0u && effects().base_roughness.y>0.0) { shade=canopy(p); }
                strength=frame.sun_color.xyz*(frame.sun_color.w*shade);
            } else if (k==1u) {
                if (sky_cdf.width==0u) { continue; }
                let light=sky_sample(vec2f(random(state),random(state)));
                direction=light.direction;
                local=transpose(transform)*direction;
                if (local.z<=0.0) { continue; }
                strength=sky_surface_radiance(direction);
                pdf=light.pdf;
                if (!(pdf>0.0)) { continue; }
            } else {
                let picked=direct_light(k-2u,transform,p,origin,geometric,state);
                if (picked.limit<0.0) { continue; }
                local=picked.wi;
                direction=picked.direction;
                strength=picked.scale;
                pdf=picked.pdf;
                limit=picked.limit;
                shape=picked.shape;
                lit.roughness=max(s.roughness,picked.roughness);
            }
            if (pdf>0.0) {
                let bsdf=bsdf_pdf(s,wo,local);
                if (k==1u) {
                    strength/=pdf+bsdf;
                } else {
                    let ratio=bsdf/pdf;
                    strength/=pdf*(1.0+ratio*ratio);
                }
            }
            if (shape>0u) {
                let i=shape-1u;
                var t=1e20;
                if (u32(shapes[i].a.w)==5u) { t=union_hit(i,origin,direction,1e20); } else { t=primitive_hit(i,origin,direction,1e20); }
                if (t<=0.0001 || t>=1e19) { continue; }
                limit=t*0.9999-0.0002;
            }
            if (limit>0.0) {
                if (TRANSMISSIVE_SHADOWS) {
                    var unshadowed=throughput*eval(lit,wo,local)*select(1.0,glossy_share(s),pdf>0.0)*local.z*strength;
                    let reach=throughput*strength;
                    let keep=clamp(16.0*PI*max(unshadowed.r,max(unshadowed.g,unshadowed.b))/max(max(reach.r,max(reach.g,reach.b)),1e-30),0.0,1.0);
                    if (keep<1.0) {
                        if (random(state)>=keep) { continue; }
                        unshadowed/=keep;
                    }
                    result+=indirect_capped(unshadowed*shadow_passed(origin,direction,limit),bounce>0u);
                    continue;
                } else if (occluded(origin,direction,limit)) { continue; }
            }
            result+=indirect_capped(throughput*eval(lit,wo,local)*select(1.0,glossy_share(s),pdf>0.0)*local.z*strength,bounce>0u);
        }
        let sample=sample_bsdf(s,wo,entering,carried,state);
        let wi=sample.dir;
        diffused=diffused || sample.lobe==0u;
        lobes+=vec3u(select(0u,1u,sample.lobe==0u),select(0u,1u,sample.lobe==1u),select(0u,1u,sample.lobe==2u));
        last=bounce>=TOTAL_BOUNCES || any(lobes>vec3u(DIFFUSE_BOUNCES,GLOSSY_BOUNCES,TRANSMISSION_BOUNCES));
        through=sample.pdf< -1.5 && lit_vertex;
        lit_vertex=sample.pdf>0.0 || through;
        if (sample.pdf<0.0) {
            throughput*=sample.weight;
            carried=sample.channel;
            if (sample.pdf< -1.5) {
                throughput*=sqrt(clamp(s.base,vec3f(0.0),vec3f(1.0)))*beer(s.thickness,s.subsurface_tint,s.absorption);
            }
            d=normalize(transform*wi);
            previous_pdf=0.0;
        } else {
            if (wi.z<=0.0 || sample.pdf<=0.0) { break; }
            throughput*=eval(s,wo,wi)*glossy_share(s)*wi.z/sample.pdf;
            d=normalize(transform*wi);
            previous_pdf=sample.pdf;
        }
        o=spawn_point(p,d,o,hit.t);
        if (bounce>=3u) {
            let keep=clamp(max(throughput.r,max(throughput.g,throughput.b)),0.05,0.95);
            if (random(state)>keep) { break; }
            throughput/=keep;
        }
    }
    return result;
}
@compute @workgroup_size(8,8)
fn main(@builtin(workgroup_id) group: vec3u, @builtin(local_invocation_id) local: vec3u) {
    let lane=local.y*8u+local.x;
    let first_lane=(frame.counts.w>>16u)&0x7fu;
    let end_lane=frame.counts.w>>23u;
    let blocks=(frame.size.y+7u)/8u;
    let block=((frame.counts.w&0xffffu)+group.y)*BLOCK_STRIDE%blocks;
    let gid=vec3u(group.x*8u+local.x,block*8u+local.y,0u);
    if (lane<first_lane || lane>=end_lane || gid.x>=frame.size.x || gid.y>=frame.size.y) { return; }
    let index=gid.y*frame.size.x+gid.x;
    var sum=Accum(vec4f(0.0),vec4f(0.0),vec4f(0.0));
    if (frame.size.z>0u) { sum=accum[index]; }
    if (sum.color.w<0.0) { return; }
    for (var sample=0u;sample<frame.size.w;sample++) {
        var first_albedo=vec4f(0.0);
        var first_normal=vec4f(0.0);
        var state=pcg(index*1973u+pcg(frame.counts.z*9277u+(frame.size.z+sample)*7919u+26699u));
        let jitter=vec2f(random(&state),random(&state));
        let ndc=(vec2f(gid.xy)+jitter)/vec2f(frame.size.xy)*2.0-1.0;
        let kind=u32(round(frame.origin.w));
        var direction=normalize(frame.forward.xyz+frame.right.xyz*(ndc.x+frame.lens.x)+frame.up.xyz*(-ndc.y+frame.lens.y));
        var origin=frame.origin.xyz;
        if (kind==1u) {
            direction=normalize(frame.forward.xyz);
            origin+=normalize(frame.right.xyz)*((ndc.x+frame.lens.x)*frame.right.w)+normalize(frame.up.xyz)*((-ndc.y+frame.lens.y)*frame.up.w);
        } else if (kind==2u) {
            let phi=(ndc.x*0.5)*2.0*PI;
            let theta=(ndc.y*0.5+0.5)*PI;
            let local=vec3f(sin(theta)*sin(phi),cos(theta),-sin(theta)*cos(phi));
            direction=normalize(normalize(frame.right.xyz)*local.x+normalize(frame.up.xyz)*local.y-normalize(frame.forward.xyz)*local.z);
        } else if (frame.lens.z>0.0) {
            let focus=origin+direction*(frame.lens.w/dot(direction,normalize(frame.forward.xyz)));
            let radius=sqrt(random(&state))*frame.lens.z;
            let phi=2.0*PI*random(&state);
            origin+=normalize(frame.right.xyz)*(radius*cos(phi))+normalize(frame.up.xyz)*(radius*sin(phi));
            direction=normalize(focus-origin);
        }
        let traced=radiance(origin,direction,&state,&first_albedo,&first_normal);
        var value=traced;
        if (effects().thick_sub_film.w>0.0) {
            let depth=scene(origin,direction,1e20,false).t;
            value=steam_composite(origin,direction,depth,traced);
        }
        sum.color=vec4f(sum.color.xyz+value,sum.color.w);
        sum.albedo+=first_albedo;
        sum.normal+=first_normal;
    }
    sum.color.w=f32(frame.size.z+frame.size.w);
    accum[index]=sum;
    let count=max(sum.color.w,1.0);
    let n=sum.normal.xyz/count;
    textureStore(output,vec2i(gid.xy),vec4f(sum.color.xyz/count,select(1.0,sum.albedo.w/count,frame.forward.w>0.0)));
    textureStore(albedo,vec2i(gid.xy),sum.albedo/count);
    textureStore(normal,vec2i(gid.xy),vec4f(select(vec3f(0.0),normalize(n),length(n)>1e-6),sum.normal.w/count));
}
