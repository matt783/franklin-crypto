use pairing::{
    Field,
    SqrtField,
    PrimeField,
    PrimeFieldRepr,
    BitIterator
};

use super::{
    JubjubEngine,
    JubjubParams,
    Unknown,
    PrimeOrder,
    montgomery
};

use rand::{
    Rng
};

use std::marker::PhantomData;

// Represents the affine point (X/Z, Y/Z) via the extended
// twisted Edwards coordinates.
pub struct Point<E: JubjubEngine, Subgroup> {
    x: E::Fr,
    y: E::Fr,
    t: E::Fr,
    z: E::Fr,
    _marker: PhantomData<Subgroup>
}

fn convert_subgroup<E: JubjubEngine, S1, S2>(from: &Point<E, S1>) -> Point<E, S2>
{
    Point {
        x: from.x,
        y: from.y,
        t: from.t,
        z: from.z,
        _marker: PhantomData
    }
}

impl<E: JubjubEngine> From<Point<E, PrimeOrder>> for Point<E, Unknown>
{
    fn from(p: Point<E, PrimeOrder>) -> Point<E, Unknown>
    {
        convert_subgroup(&p)
    }
}

impl<E: JubjubEngine, Subgroup> Clone for Point<E, Subgroup>
{
    fn clone(&self) -> Self {
        convert_subgroup(self)
    }
}

impl<E: JubjubEngine, Subgroup> PartialEq for Point<E, Subgroup> {
    fn eq(&self, other: &Point<E, Subgroup>) -> bool {
        // p1 = (x1/z1, y1/z1)
        // p2 = (x2/z2, y2/z2)
        // Deciding that these two points are equal is a matter of
        // determining that x1/z1 = x2/z2, or equivalently that
        // x1*z2 = x2*z1, and similarly for y.

        let mut x1 = self.x;
        x1.mul_assign(&other.z);

        let mut y1 = self.y;
        y1.mul_assign(&other.z);

        let mut x2 = other.x;
        x2.mul_assign(&self.z);

        let mut y2 = other.y;
        y2.mul_assign(&self.z);

        x1 == x2 && y1 == y2
    }
}

impl<E: JubjubEngine> Point<E, Unknown> {
    /// This guarantees the point is in the prime order subgroup
    pub fn mul_by_cofactor(&self, params: &E::Params) -> Point<E, PrimeOrder>
    {
        let tmp = self.double(params)
                      .double(params)
                      .double(params);

        convert_subgroup(&tmp)
    }

    pub fn rand<R: Rng>(rng: &mut R, params: &E::Params) -> Self
    {
        loop {
            // given an x on the curve, y^2 = (1 + x^2) / (1 - dx^2)
            let x: E::Fr = rng.gen();
            let mut x2 = x;
            x2.square();

            let mut num = E::Fr::one();
            num.add_assign(&x2);

            x2.mul_assign(params.edwards_d());

            let mut den = E::Fr::one();
            den.sub_assign(&x2);

            match den.inverse() {
                Some(invden) => {
                    num.mul_assign(&invden);

                    match num.sqrt() {
                        Some(mut y) => {
                            if y.into_repr().is_odd() != rng.gen() {
                                y.negate();
                            }

                            let mut t = x;
                            t.mul_assign(&y);

                            return Point {
                                x: x,
                                y: y,
                                t: t,
                                z: E::Fr::one(),
                                _marker: PhantomData
                            }
                        },
                        None => {}
                    }
                },
                None => {}
            }
        }
    }
}

impl<E: JubjubEngine, Subgroup> Point<E, Subgroup> {
    /// Convert from a Montgomery point
    pub fn from_montgomery(
        m: &montgomery::Point<E, Subgroup>,
        params: &E::Params
    ) -> Self
    {
        match m.into_xy() {
            None => {
                // Map the point at infinity to the neutral element.
                Point::zero()
            },
            Some((x, y)) => {
                // The map from a Montgomery curve is defined as:
                // (x, y) -> (u, v) where
                //      u = x / y
                //      v = (x - 1) / (x + 1)
                //
                // This map is not defined for y = 0 and x = -1.
                //
                // y = 0 is a valid point only for x = 0:
                //     y^2 = x^3 + A.x^2 + x
                //       0 = x^3 + A.x^2 + x
                //       0 = x(x^2 + A.x + 1)
                // We have: x = 0  OR  x^2 + A.x + 1 = 0
                //       x^2 + A.x + 1 = 0
                //         (2.x + A)^2 = A^2 - 4 (Complete the square.)
                // The left hand side is a square, and so if A^2 - 4
                // is nonsquare, there is no solution. Indeed, A^2 - 4
                // is nonsquare.
                //
                // (0, 0) is a point of order 2, and so we map it to
                // (0, -1) in the twisted Edwards curve, which is the
                // only point of order 2 that is not the neutral element.
                if y.is_zero() {
                    // This must be the point (0, 0) as above.
                    let mut neg1 = E::Fr::one();
                    neg1.negate();

                    Point {
                        x: E::Fr::zero(),
                        y: neg1,
                        t: E::Fr::zero(),
                        z: E::Fr::one(),
                        _marker: PhantomData
                    }
                } else {
                    // Otherwise, as stated above, the mapping is still
                    // not defined at x = -1. However, x = -1 is not
                    // on the curve when A - 2 is nonsquare:
                    //     y^2 = x^3 + A.x^2 + x
                    //     y^2 = (-1) + A + (-1)
                    //     y^2 = A - 2
                    // Indeed, A - 2 is nonsquare.
                    //
                    // We need to map into (projective) extended twisted
                    // Edwards coordinates (X, Y, T, Z) which represents
                    // the point (X/Z, Y/Z) with Z nonzero and T = XY/Z.
                    //
                    // Thus, we compute...
                    //
                    // u = x(x + 1)
                    // v = y(x - 1)
                    // t = x(x - 1)
                    // z = y(x + 1)  (Cannot be nonzero, as above.)
                    //
                    // ... which represents the point ( x / y , (x - 1) / (x + 1) )
                    // as required by the mapping and preserves the property of
                    // the auxillary coordinate t.
                    //
                    // We need to scale the coordinate, so u and t will have
                    // an extra factor s.

                    // u = xs
                    let mut u = x;
                    u.mul_assign(params.scale());

                    // v = x - 1
                    let mut v = x;
                    v.sub_assign(&E::Fr::one());

                    // t = xs(x - 1)
                    let mut t = u;
                    t.mul_assign(&v);

                    // z = (x + 1)
                    let mut z = x;
                    z.add_assign(&E::Fr::one());

                    // u = xs(x + 1)
                    u.mul_assign(&z);

                    // z = y(x + 1)
                    z.mul_assign(&y);

                    // v = y(x - 1)
                    v.mul_assign(&y);

                    Point {
                        x: u,
                        y: v,
                        t: t,
                        z: z,
                        _marker: PhantomData
                    }
                }
            }
        }
    }

    /// Attempts to cast this as a prime order element, failing if it's
    /// not in the prime order subgroup.
    pub fn as_prime_order(&self, params: &E::Params) -> Option<Point<E, PrimeOrder>> {
        if self.mul(E::Fs::char(), params) == Point::zero() {
            Some(convert_subgroup(self))
        } else {
            None
        }
    }

    pub fn zero() -> Self {
        Point {
            x: E::Fr::zero(),
            y: E::Fr::one(),
            t: E::Fr::zero(),
            z: E::Fr::one(),
            _marker: PhantomData
        }
    }

    pub fn into_xy(&self) -> (E::Fr, E::Fr)
    {
        let zinv = self.z.inverse().unwrap();

        let mut x = self.x;
        x.mul_assign(&zinv);

        let mut y = self.y;
        y.mul_assign(&zinv);

        (x, y)
    }

    pub fn negate(&self) -> Self {
        let mut p = self.clone();

        p.x.negate();
        p.t.negate();

        p
    }

    pub fn double(&self, params: &E::Params) -> Self {
        self.add(self, params)
    }

    pub fn add(&self, other: &Self, params: &E::Params) -> Self
    {
        // A = x1 * x2
        let mut a = self.x;
        a.mul_assign(&other.x);

        // B = y1 * y2
        let mut b = self.y;
        b.mul_assign(&other.y);

        // C = d * t1 * t2
        let mut c = params.edwards_d().clone();
        c.mul_assign(&self.t);
        c.mul_assign(&other.t);

        // D = z1 * z2
        let mut d = self.z;
        d.mul_assign(&other.z);

        // H = B - aA
        //   = B + A
        let mut h = b;
        h.add_assign(&a);

        // E = (x1 + y1) * (x2 + y2) - A - B
        //   = (x1 + y1) * (x2 + y2) - H
        let mut e = self.x;
        e.add_assign(&self.y);
        {
            let mut tmp = other.x;
            tmp.add_assign(&other.y);
            e.mul_assign(&tmp);
        }
        e.sub_assign(&h);

        // F = D - C
        let mut f = d;
        f.sub_assign(&c);

        // G = D + C
        let mut g = d;
        g.add_assign(&c);

        // x3 = E * F
        let mut x3 = e;
        x3.mul_assign(&f);

        // y3 = G * H
        let mut y3 = g;
        y3.mul_assign(&h);

        // t3 = E * H
        let mut t3 = e;
        t3.mul_assign(&h);

        // z3 = F * G
        let mut z3 = f;
        z3.mul_assign(&g);

        Point {
            x: x3,
            y: y3,
            t: t3,
            z: z3,
            _marker: PhantomData
        }
    }

    pub fn mul<S: Into<<E::Fs as PrimeField>::Repr>>(
        &self,
        scalar: S,
        params: &E::Params
    ) -> Self
    {
        let mut res = Self::zero();

        for b in BitIterator::new(scalar.into()) {
            res = res.double(params);

            if b {
                res = res.add(self, params);
            }
        }

        res
    }
}