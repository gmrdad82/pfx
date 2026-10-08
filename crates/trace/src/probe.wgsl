@group(0) @binding(17) var probe_depth: texture_storage_2d<rgba32float, write>;
@group(0) @binding(18) var probe_normal: texture_storage_2d<rgba32float, write>;
@compute @workgroup_size(8,8)
fn probe(@builtin(global_invocation_id) gid: vec3u) {
    _ = triangles[0];
    _ = nodes[0];
    _ = shapes[0];
    _ = materials[0].tail;
    _ = triangle_uv_v[0];
    _ = content_texels[0];
    _ = content_info[0];
    _ = instance_surfaces[0];
    _ = content_layers[0];
    if (gid.x>=frame.size.x || gid.y>=frame.size.y) { return; }
    let ndc=(vec2f(gid.xy)+vec2f(0.5))/vec2f(frame.size.xy)*2.0-1.0;
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
    }
    let hit=scene(origin,direction,1e20,false);
    if (hit.t>=1e19) {
        textureStore(probe_depth,vec2i(gid.xy),vec4f(0.0));
        textureStore(probe_normal,vec2i(gid.xy),vec4f(0.0));
        return;
    }
    let n=select(hit.n,-hit.n,dot(hit.n,direction)>0.0);
    var depth=hit.t;
    if (kind!=2u) { depth=hit.t*dot(direction,normalize(frame.forward.xyz)); }
    textureStore(probe_depth,vec2i(gid.xy),vec4f(depth,f32(hit.material+1u),hit.t,1.0));
    textureStore(probe_normal,vec2i(gid.xy),vec4f(n,0.0));
}
