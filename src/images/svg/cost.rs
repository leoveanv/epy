use super::SvgDecodeOptions;
use super::validation::svg_error;
use crate::Result;
use resvg::usvg;

#[derive(Default)]
struct RenderCost {
    scratch_units: f64,
    work_units: f64,
    nodes: usize,
}

pub(super) fn render_scale(tree: &usvg::Tree, options: &SvgDecodeOptions) -> Result<f32> {
    let width = f64::from(tree.size().width());
    let height = f64::from(tree.size().height());
    if width.max(height) > f64::from(options.image_limits.dimension_px_max) {
        return Err(svg_error("dimension limit exceeded"));
    }
    let mut cost = RenderCost::default();
    cost.measure(tree, options)?;
    let pixels_max = options
        .limits
        .render_pixels_max
        .min(options.image_limits.pixels_max);
    let scratch_max = options
        .limits
        .scratch_bytes_max
        .min(options.image_limits.decoded_bytes_max);
    if pixels_max == 0 || scratch_max == 0 || options.limits.work_pixels_max == 0 {
        return Err(svg_error("render budget is empty"));
    }
    let scale = 1.0_f64
        .min((pixels_max as f64 / (width * height)).sqrt())
        .min((scratch_max as f64 / cost.scratch_units.max(1.0)).sqrt())
        .min((options.limits.work_pixels_max as f64 / cost.work_units.max(1.0)).sqrt());
    if !scale.is_finite() || width.min(height) * scale < 1.0 {
        return Err(svg_error("render complexity limit exceeded"));
    }
    Ok(scale as f32)
}

impl RenderCost {
    fn measure(&mut self, tree: &usvg::Tree, options: &SvgDecodeOptions) -> Result<()> {
        self.measure_group(tree.root(), tree, options, 0)
    }

    fn measure_group(
        &mut self,
        group: &usvg::Group,
        tree: &usvg::Tree,
        options: &SvgDecodeOptions,
        depth: usize,
    ) -> Result<()> {
        if depth > options.limits.depth_max.saturating_mul(4) {
            return Err(svg_error("render depth limit exceeded"));
        }
        options.cancellation.check()?;
        self.nodes = self.nodes.saturating_add(group.children().len() + 1);
        if self.nodes > options.limits.nodes_max {
            return Err(svg_error("expanded render node limit exceeded"));
        }
        let filter_count = group
            .filters()
            .iter()
            .map(|filter| filter.primitives().len())
            .sum::<usize>();
        let area = bounded_area(group.abs_layer_bounding_box(), tree);
        // #NOTE: Scratch estimates include intermediate filter/layer surfaces, not just the final canvas.
        self.scratch_units += area * (filter_count as f64 + 4.0) * 32.0;
        self.work_units += area * (filter_count as f64 + 1.0) * 64.0;
        for node in group.children() {
            if let Some(bounds) = node.abs_layer_bounding_box() {
                self.work_units += bounded_area(bounds, tree) * 16.0;
            }
            if let usvg::Node::Group(group) = node {
                self.measure_group(group, tree, options, depth + 1)?;
            }
            let mut result = Ok(());
            node.subroots(|root| {
                if result.is_ok() {
                    result = self.measure_group(root, tree, options, depth + 1);
                }
            });
            result?;
        }
        Ok(())
    }
}

fn bounded_area(bounds: usvg::NonZeroRect, tree: &usvg::Tree) -> f64 {
    let width = f64::from(bounds.width()).min(f64::from(tree.size().width()) * 5.0);
    let height = f64::from(bounds.height()).min(f64::from(tree.size().height()) * 5.0);
    width * height
}
