pub fn apply(pass: &mut wgpu::RenderPass<'_>, size: [u32; 2]) {
    pass.set_viewport(0.0, 0.0, size[0] as f32, size[1] as f32, 0.0, 1.0);
    pass.set_scissor_rect(0, 0, size[0], size[1]);
}

pub fn check(size: [u32; 2], capacity: [u32; 2]) -> Result<(), String> {
    if size[0] == 0 || size[1] == 0 || size[0] > capacity[0] || size[1] > capacity[1] {
        return Err(format!(
            "the viewport {}x{} must be at least 1 by 1 and inside the targets' {}x{}",
            size[0], size[1], capacity[0], capacity[1]
        ));
    }
    Ok(())
}

pub fn mip(size: u32, level: u32) -> u32 {
    (size >> level).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_viewport_must_fit_its_targets() {
        assert!(check([4, 3], [4, 3]).is_ok());
        assert!(check([1, 1], [4, 3]).is_ok());
        assert!(check([0, 3], [4, 3]).is_err());
        assert!(check([5, 3], [4, 3]).is_err());
        assert!(check([4, 4], [4, 3]).is_err());
        assert_eq!(mip(960, 3), 120);
        assert_eq!(mip(5, 3), 1);
    }
}
