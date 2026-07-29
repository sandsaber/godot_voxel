//! Image-based 2D heightmap lookup for the voxel graph.
//!
//! Ports `NODE_IMAGE_2D` from the C++ graph generator. An `Image2D` stores
//! a 2D grid of f32 values (typically heightmap data) and samples them with
//! bilinear interpolation at arbitrary `(x, z)` coordinates.

/// A 2D image used by the graph `Image2D` node. Stores f32 values in a
/// row-major grid. Sampling wraps or clamps at edges.
#[derive(Debug, Clone)]
pub struct Image2D {
    /// Width of the image grid.
    width: u32,
    /// Height of the image grid.
    height: u32,
    /// Row-major pixel data (height rows × width columns).
    data: Vec<f32>,
}

impl Image2D {
    /// Create a new image filled with `fill_value`.
    pub fn new_filled(width: u32, height: u32, fill_value: f32) -> Self {
        Self {
            width,
            height,
            data: vec![fill_value; (width * height) as usize],
        }
    }

    /// Create a new image from raw f32 data.
    pub fn from_data(width: u32, height: u32, data: Vec<f32>) -> Self {
        assert_eq!(data.len(), (width * height) as usize);
        Self {
            width,
            height,
            data,
        }
    }

    /// Image width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Get the raw value at integer coordinates `(x, y)`. Returns 0.0 if
    /// out of bounds.
    pub fn get_pixel(&self, x: u32, y: u32) -> f32 {
        if x >= self.width || y >= self.height {
            return 0.0;
        }
        self.data[(y * self.width + x) as usize]
    }

    /// Set the raw value at integer coordinates `(x, y)`.
    pub fn set_pixel(&mut self, x: u32, y: u32, value: f32) {
        if x < self.width && y < self.height {
            self.data[(y * self.width + x) as usize] = value;
        }
    }

    /// Sample the image with bilinear interpolation at floating-point
    /// coordinates `(fx, fy)`. Coordinates outside the image are clamped
    /// to the edge. Matches C++ `Image::get_pixel_bilinear` behavior.
    pub fn sample_bilinear(&self, fx: f32, fy: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        if self.width == 1 && self.height == 1 {
            return self.data[0];
        }

        // Clamp to valid range.
        let cx = fx.clamp(0.0, (self.width - 1) as f32);
        let cy = fy.clamp(0.0, (self.height - 1) as f32);

        let x0 = cx.floor() as u32;
        let y0 = cy.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);

        let tx = cx - x0 as f32;
        let ty = cy - y0 as f32;

        let v00 = self.get_pixel(x0, y0);
        let v10 = self.get_pixel(x1, y0);
        let v01 = self.get_pixel(x0, y1);
        let v11 = self.get_pixel(x1, y1);

        let a = v00 * (1.0 - tx) + v10 * tx;
        let b = v01 * (1.0 - tx) + v11 * tx;
        a * (1.0 - ty) + b * ty
    }

    /// Returns the min and max values in the image.
    pub fn value_range(&self) -> (f32, f32) {
        if self.data.is_empty() {
            return (0.0, 0.0);
        }
        let mut min_val = f32::INFINITY;
        let mut max_val = f32::NEG_INFINITY;
        for &v in &self.data {
            if v < min_val {
                min_val = v;
            }
            if v > max_val {
                max_val = v;
            }
        }
        (min_val, max_val)
    }
}

impl Default for Image2D {
    fn default() -> Self {
        Self::new_filled(1, 1, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_image_samples_constant() {
        let img = Image2D::new_filled(16, 16, 0.5);
        assert!((img.sample_bilinear(3.7, 7.2) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn gradient_image_interpolates() {
        let img = Image2D::from_data(2, 1, vec![0.0, 1.0]);
        // At x=0.5, should be 0.5.
        assert!((img.sample_bilinear(0.5, 0.0) - 0.5).abs() < 1e-5);
        // At x=0.25, should be 0.25.
        assert!((img.sample_bilinear(0.25, 0.0) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn out_of_bounds_clamps_to_edge() {
        let img = Image2D::from_data(
            4,
            4,
            vec![
                0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0,
                15.0,
            ],
        );
        // Negative x clamps to 0.
        assert!((img.sample_bilinear(-5.0, 0.0) - 0.0).abs() < 1e-5);
        // Large x clamps to last column.
        assert!((img.sample_bilinear(100.0, 0.0) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn value_range_uniform() {
        let img = Image2D::new_filled(8, 8, 0.5);
        let (min, max) = img.value_range();
        assert!((min - 0.5).abs() < 1e-5);
        assert!((max - 0.5).abs() < 1e-5);
    }

    #[test]
    fn value_range_varied() {
        let img = Image2D::from_data(2, 2, vec![0.1, 0.9, 0.3, 0.7]);
        let (min, max) = img.value_range();
        assert!((min - 0.1).abs() < 1e-5);
        assert!((max - 0.9).abs() < 1e-5);
    }

    #[test]
    fn set_pixel_changes_value() {
        let mut img = Image2D::new_filled(4, 4, 0.0);
        img.set_pixel(2, 3, 0.75);
        assert!((img.get_pixel(2, 3) - 0.75).abs() < 1e-5);
    }

    #[test]
    fn single_pixel_image() {
        let img = Image2D::new_filled(1, 1, 0.42);
        assert!((img.sample_bilinear(0.0, 0.0) - 0.42).abs() < 1e-5);
    }
}
