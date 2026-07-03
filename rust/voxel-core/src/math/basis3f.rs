//! 3×3 rotation/scale matrix (32-bit float).
//!
//! Ported from `util/math/basis3f.h`. Stores three row vectors (transposed vs
//! column vectors, matching Godot's `Basis` storage). Used by [`super::transform3f`].

use super::funcs;
use super::quaternion::{math as qm, Quaternionf};
use super::vector3::{math as v3, Vector3f};

/// 3×3 matrix specialized for 3D bases. Matches C++ `Basis3f`. Default is identity.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct Basis3f {
    /// Rows (the matrix is stored transposed relative to its column vectors, as
    /// in Godot's `Basis`). `rows[r][c]` is element `(r,c)`.
    pub rows: [Vector3f; 3],
}

impl Default for Basis3f {
    #[inline]
    fn default() -> Self {
        Self::identity()
    }
}

impl Basis3f {
    pub const fn identity() -> Self {
        Self {
            rows: [
                Vector3f::new(1.0, 0.0, 0.0),
                Vector3f::new(0.0, 1.0, 0.0),
                Vector3f::new(0.0, 0.0, 1.0),
            ],
        }
    }

    /// Build from column vectors (x/y/z axes). Matches the axis-argument ctor.
    pub fn from_axes(x_axis: Vector3f, y_axis: Vector3f, z_axis: Vector3f) -> Self {
        let mut b = Self::identity();
        b.set_column(0, x_axis);
        b.set_column(1, y_axis);
        b.set_column(2, z_axis);
        b
    }

    /// Build from a unit quaternion. Matches `Basis3f(Quaternionf)`.
    pub fn from_quaternion(q: Quaternionf) -> Self {
        let mut b = Self::identity();
        b.set_quaternion(q);
        b
    }

    pub fn set_quaternion(&mut self, q: Quaternionf) {
        let d = qm::length_squared(q);
        let s = 2.0 / d;
        let (xs, ys, zs) = (q.x * s, q.y * s, q.z * s);
        let (wx, wy, wz) = (q.w * xs, q.w * ys, q.w * zs);
        let (xx, xy, xz) = (q.x * xs, q.x * ys, q.x * zs);
        let (yy, yz, zz) = (q.y * ys, q.y * zs, q.z * zs);

        self.rows[0][0] = 1.0 - (yy + zz);
        self.rows[0][1] = xy - wz;
        self.rows[0][2] = xz + wy;

        self.rows[1][0] = xy + wz;
        self.rows[1][1] = 1.0 - (xx + zz);
        self.rows[1][2] = yz - wx;

        self.rows[2][0] = xz - wy;
        self.rows[2][1] = yz + wx;
        self.rows[2][2] = 1.0 - (xx + yy);
    }

    /// Rotation matrix from a unit axis and its angle's cosine/sine.
    /// Matches `set_axis_angle`.
    pub fn set_axis_angle(&mut self, axis: Vector3f, cosine: f32, sine: f32) {
        let axis_sq = Vector3f::new(axis.x * axis.x, axis.y * axis.y, axis.z * axis.z);
        self.rows[0][0] = axis_sq.x + cosine * (1.0 - axis_sq.x);
        self.rows[1][1] = axis_sq.y + cosine * (1.0 - axis_sq.y);
        self.rows[2][2] = axis_sq.z + cosine * (1.0 - axis_sq.z);

        let t = 1.0 - cosine;

        let mut xyzt = axis.x * axis.y * t;
        let mut zyxs = axis.z * sine;
        self.rows[0][1] = xyzt - zyxs;
        self.rows[1][0] = xyzt + zyxs;

        xyzt = axis.x * axis.z * t;
        zyxs = axis.y * sine;
        self.rows[0][2] = xyzt + zyxs;
        self.rows[2][0] = xyzt - zyxs;

        xyzt = axis.y * axis.z * t;
        zyxs = axis.x * sine;
        self.rows[1][2] = xyzt - zyxs;
        self.rows[2][1] = xyzt + zyxs;
    }

    #[inline]
    pub fn set_column(&mut self, index: usize, value: Vector3f) {
        debug_assert!(index < 3);
        self.rows[0][index] = value.x;
        self.rows[1][index] = value.y;
        self.rows[2][index] = value.z;
    }

    #[inline]
    pub fn get_column(&self, index: usize) -> Vector3f {
        debug_assert!(index < 3);
        Vector3f::new(
            self.rows[0][index],
            self.rows[1][index],
            self.rows[2][index],
        )
    }

    /// Matrix × column vector. Matches `xform`.
    #[inline]
    pub fn xform(&self, v: Vector3f) -> Vector3f {
        Vector3f::new(
            v3::dot(self.rows[0], v),
            v3::dot(self.rows[1], v),
            v3::dot(self.rows[2], v),
        )
    }

    pub fn scale(&mut self, s: f32) {
        for r in 0..3 {
            for c in 0..3 {
                self.rows[r][c] *= s;
            }
        }
    }

    #[inline]
    pub fn scaled(&self, s: f32) -> Basis3f {
        let mut b = *self;
        b.scale(s);
        b
    }

    /// Absolute per-column scale. Matches `get_scale_abs`.
    pub fn get_scale_abs(&self) -> Vector3f {
        Vector3f::new(
            v3::length(Vector3f::new(
                self.rows[0][0],
                self.rows[1][0],
                self.rows[2][0],
            )),
            v3::length(Vector3f::new(
                self.rows[0][1],
                self.rows[1][1],
                self.rows[2][1],
            )),
            v3::length(Vector3f::new(
                self.rows[0][2],
                self.rows[1][2],
                self.rows[2][2],
            )),
        )
    }

    /// In-place Gram-Schmidt orthonormalization. Matches `orthonormalize`.
    pub fn orthonormalize(&mut self) {
        let mut x = self.get_column(0);
        let mut y = self.get_column(1);
        let mut z = self.get_column(2);

        x = v3::normalized(x);
        y = y - x * v3::dot(x, y);
        y = v3::normalized(y);
        z = z - x * v3::dot(x, z) - y * v3::dot(y, z);
        z = v3::normalized(z);

        self.set_column(0, x);
        self.set_column(1, y);
        self.set_column(2, z);
    }

    #[inline]
    pub fn orthonormalized(&self) -> Basis3f {
        let mut b = *self;
        b.orthonormalize();
        b
    }

    #[inline]
    pub fn determinant(&self) -> f32 {
        self.rows[0][0] * (self.rows[1][1] * self.rows[2][2] - self.rows[2][1] * self.rows[1][2])
            - self.rows[1][0]
                * (self.rows[0][1] * self.rows[2][2] - self.rows[2][1] * self.rows[0][2])
            + self.rows[2][0]
                * (self.rows[0][1] * self.rows[1][2] - self.rows[1][1] * self.rows[0][2])
    }

    /// Quaternion from a (possibly unnormalized) basis. Matches `get_quaternion`.
    pub fn get_quaternion(&self) -> Quaternionf {
        let m = *self;
        let trace = m.rows[0][0] + m.rows[1][1] + m.rows[2][2];

        if trace > 0.0 {
            let s = funcs::sqrt_f32(trace + 1.0);
            let w = s * 0.5;
            let inv = 0.5 / s;
            return Quaternionf::new(
                (m.rows[2][1] - m.rows[1][2]) * inv,
                (m.rows[0][2] - m.rows[2][0]) * inv,
                (m.rows[1][0] - m.rows[0][1]) * inv,
                w,
            );
        }

        let i = if m.rows[0][0] < m.rows[1][1] {
            if m.rows[1][1] < m.rows[2][2] {
                2
            } else {
                1
            }
        } else if m.rows[0][0] < m.rows[2][2] {
            2
        } else {
            0
        };
        let j = (i + 1) % 3;
        let k = (i + 2) % 3;

        let mut s = funcs::sqrt_f32(m.rows[i][i] - m.rows[j][j] - m.rows[k][k] + 1.0);
        let inv = 0.5 / s;
        s *= 0.5;
        let mut t = [0.0f32; 4];
        t[i] = s;
        t[3] = (m.rows[k][j] - m.rows[j][k]) * inv;
        t[j] = (m.rows[j][i] + m.rows[i][j]) * inv;
        t[k] = (m.rows[k][i] + m.rows[i][k]) * inv;
        Quaternionf::new(t[0], t[1], t[2], t[3])
    }

    /// Rotation quaternion after orthonormalizing. Matches `get_rotation_quaternion`.
    pub fn get_rotation_quaternion(&self) -> Quaternionf {
        let mut m = self.orthonormalized();
        let det = m.determinant();
        if det < 0.0 {
            m.scale(-1.0);
        }
        m.get_quaternion()
    }
}

/// Rotate `v` around `axis` by the angle with the given cosine/sine. Matches `math::rotated`.
pub fn rotated(v: Vector3f, axis: Vector3f, cosine: f32, sine: f32) -> Vector3f {
    let mut b = Basis3f::identity();
    b.set_axis_angle(axis, cosine, sine);
    b.xform(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_xform_is_identity() {
        let b = Basis3f::identity();
        let v = Vector3f::new(1.0, 2.0, 3.0);
        assert_eq!(b.xform(v), v);
        assert!((b.determinant() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn column_roundtrip() {
        let mut b = Basis3f::identity();
        b.set_column(1, Vector3f::new(4.0, 5.0, 6.0));
        assert_eq!(b.get_column(1), Vector3f::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn quaternion_identity_gives_identity_basis() {
        let b = Basis3f::from_quaternion(Quaternionf::IDENTITY);
        assert_eq!(b, Basis3f::identity());
    }

    #[test]
    fn scaled_uniformly() {
        let b = Basis3f::identity().scaled(2.0);
        assert_eq!(b.get_scale_abs(), Vector3f::new(2.0, 2.0, 2.0));
    }

    #[test]
    fn orthonormalized_rotation_is_normalized() {
        // A scaled identity orthonormalized returns to unit scale.
        let b = Basis3f::identity().scaled(3.0).orthonormalized();
        let diff = b.get_scale_abs() - Vector3f::new(1.0, 1.0, 1.0);
        assert!(v3::length(diff) < 1e-5);
    }
}
