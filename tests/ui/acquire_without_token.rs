use madori::Surface as _;

fn draw(surface: &wgpu::Surface<'static>) {
    let _ = surface.acquire();
}

fn main() {
    let _ = draw;
}
