//! Camera interpolation for interactive playback and exported frames.
use pyo3::{exceptions::PyValueError, prelude::*};
use std::f64::consts::{PI, TAU};

#[pyclass(from_py_object, frozen, get_all, eq, module = "built_core")]
#[derive(Clone, Copy, PartialEq)]
pub struct Pose {
    target: (f64, f64, f64),
    distance: f64,
    yaw: f64,
    pitch: f64,
    fov_deg: f64,
    perspective: bool,
    two_point: bool,
}

#[pymethods]
impl Pose {
    #[new]
    #[pyo3(signature=(target, distance, yaw, pitch, fov_deg=45.0, perspective=true, two_point=false))]
    fn new(
        target: (f64, f64, f64),
        distance: f64,
        yaw: f64,
        pitch: f64,
        fov_deg: f64,
        perspective: bool,
        two_point: bool,
    ) -> Self {
        Self {
            target,
            distance,
            yaw,
            pitch,
            fov_deg,
            perspective,
            two_point,
        }
    }

    #[staticmethod]
    fn of_camera(camera: &Bound<'_, PyAny>) -> PyResult<Self> {
        let t = camera.getattr("target")?;
        Self::read(
            camera,
            (
                t.call_method0("x")?.extract()?,
                t.call_method0("y")?.extract()?,
                t.call_method0("z")?.extract()?,
            ),
        )
    }

    #[staticmethod]
    fn of_view(view: &Bound<'_, PyAny>) -> PyResult<Self> {
        let [x, y, z] = view.getattr("target")?.extract::<[f64; 3]>()?;
        Self::read(view, (x, y, z))
    }

    fn put_on(&self, camera: &Bound<'_, PyAny>) -> PyResult<()> {
        let point = camera
            .py()
            .import("built_core")?
            .call_method1("Vec3", (self.target,))?;
        camera.setattr("target", point)?;
        camera.setattr("distance", self.distance)?;
        camera.setattr("yaw", self.yaw)?;
        camera.setattr("pitch", self.pitch)?;
        camera.setattr("fov_deg", self.fov_deg)?;
        camera.setattr("perspective", self.perspective)?;
        camera.setattr("two_point", self.two_point)
    }

    #[pyo3(signature=(other, tol=1e-6))]
    fn close_to(&self, other: &Pose, tol: f64) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() <= tol * a.abs().max(b.abs()).max(1.0);
        close(self.target.0, other.target.0)
            && close(self.target.1, other.target.1)
            && close(self.target.2, other.target.2)
            && (self.distance - other.distance).abs() <= tol * self.distance.max(1.0)
            && turn(self.yaw, other.yaw).abs() <= tol
            && (self.pitch - other.pitch).abs() <= tol
            && (self.fov_deg - other.fov_deg).abs() <= tol
            && self.perspective == other.perspective
            && self.two_point == other.two_point
    }
}
impl Pose {
    fn read(obj: &Bound<'_, PyAny>, target: (f64, f64, f64)) -> PyResult<Self> {
        Ok(Self {
            target,
            distance: obj.getattr("distance")?.extract()?,
            yaw: obj.getattr("yaw")?.extract()?,
            pitch: obj.getattr("pitch")?.extract()?,
            fov_deg: obj.getattr("fov_deg")?.extract()?,
            perspective: obj.getattr("perspective")?.is_truthy()?,
            two_point: obj.hasattr("two_point")? && obj.getattr("two_point")?.is_truthy()?,
        })
    }
}

#[pyfunction]
fn ease(u: f64) -> f64 {
    0.5 - 0.5 * (PI * u.clamp(0., 1.)).cos()
}
#[pyfunction]
fn turn(a: f64, b: f64) -> f64 {
    let delta = (b - a).rem_euclid(TAU);
    if delta > PI {
        delta - TAU
    } else {
        delta
    }
}
#[pyfunction]
fn between(a: &Pose, b: &Pose, u: f64) -> Pose {
    if u <= 0. {
        return *a;
    }
    if u >= 1. {
        return *b;
    }
    let lerp = |x: f64, y: f64| x + (y - x) * u;
    Pose {
        target: (
            lerp(a.target.0, b.target.0),
            lerp(a.target.1, b.target.1),
            lerp(a.target.2, b.target.2),
        ),
        distance: lerp(a.distance.max(1e-9).ln(), b.distance.max(1e-9).ln()).exp(),
        yaw: a.yaw + turn(a.yaw, b.yaw) * u,
        pitch: lerp(a.pitch, b.pitch),
        fov_deg: lerp(a.fov_deg, b.fov_deg),
        perspective: a.perspective || b.perspective,
        two_point: a.two_point && b.two_point,
    }
}
#[pyfunction]
fn glide(a: &Pose, b: &Pose, seconds: f64, t: f64) -> Pose {
    if seconds <= 0. || t >= seconds {
        *b
    } else {
        between(a, b, ease(t / seconds))
    }
}

#[pyclass(get_all, module = "built_core", name = "SceneRun")]
struct Run {
    start: Pose,
    poses: Vec<Pose>,
    transition: f64,
    pause: f64,
    r#loop: bool,
}
#[pymethods]
impl Run {
    #[new]
    #[pyo3(signature=(start, poses, transition, pause, r#loop=true))]
    fn new(start: Pose, poses: Vec<Pose>, transition: f64, pause: f64, r#loop: bool) -> Self {
        Self {
            start,
            poses,
            transition: transition.max(0.),
            pause: pause.max(0.),
            r#loop,
        }
    }
    #[getter]
    fn leg(&self) -> f64 {
        self.transition + self.pause
    }
    fn duration(&self) -> f64 {
        self.leg() * self.poses.len() as f64
    }
    fn at(&self, t: f64) -> (Pose, isize) {
        sample(
            self.start,
            &self.poses,
            self.transition,
            self.pause,
            self.r#loop,
            t,
        )
    }
}
fn sample(
    start: Pose,
    poses: &[Pose],
    transition: f64,
    pause: f64,
    looping: bool,
    t: f64,
) -> (Pose, isize) {
    let n = poses.len();
    if n == 0 {
        return (start, -1);
    }
    let leg = transition + pause;
    if leg <= 0. {
        return (poses[0], 0);
    }
    let t = t.max(0.);
    let k = (t / leg).floor() as usize;
    if !looping && k >= n {
        return (poses[n - 1], (n - 1) as isize);
    }
    let into = t - k as f64 * leg;
    let i = k % n;
    if into >= transition {
        return (poses[i], i as isize);
    }
    if k == 0 {
        return (glide(&start, &poses[0], transition, into), -1);
    }
    let last = (i + n - 1) % n;
    (
        glide(&poses[last], &poses[i], transition, into),
        last as isize,
    )
}
#[pyfunction]
pub fn video_seconds(count: usize, transition: f64, pause: f64) -> f64 {
    if count == 0 {
        return 0.;
    }
    pause.max(0.) + (count - 1) as f64 * (transition.max(0.) + pause.max(0.))
}
#[pyfunction]
pub fn video_frame_count(count: usize, transition: f64, pause: f64, fps: u32) -> usize {
    if count == 0 {
        0
    } else {
        (video_seconds(count, transition, pause) * fps as f64).round_ties_even() as usize + 1
    }
}
#[pyfunction]
fn video_at(poses: Vec<Pose>, transition: f64, pause: f64, t: f64) -> PyResult<(Pose, isize)> {
    Ok(VideoTimeline::new(poses, transition, pause)?.at(t))
}

#[pyclass(module = "built_core")]
struct VideoTimeline {
    run: Run,
}

#[pymethods]
impl VideoTimeline {
    #[new]
    fn new(mut poses: Vec<Pose>, transition: f64, pause: f64) -> PyResult<Self> {
        if poses.is_empty() {
            return Err(PyValueError::new_err("a video needs at least one scene"));
        }
        let first = poses.remove(0);
        Ok(Self {
            run: Run::new(first, poses, transition, pause, false),
        })
    }

    fn at(&self, t: f64) -> (Pose, isize) {
        if t < self.run.pause || self.run.poses.is_empty() {
            return (self.run.start, 0);
        }
        let (pose, index) = self.run.at(t - self.run.pause);
        (pose, index + 1)
    }
}
enum Motion {
    Glide { from: Pose, to: Pose, duration: f64 },
    Play(Run),
}

/// The host supplies monotonic time and owns the scene handles. Each arrival
/// is delivered once, after the camera pose for that frame has been applied.
#[pyclass(module = "built_core")]
#[derive(Default)]
struct ScenePlayback {
    motion: Option<Motion>,
    started: f64,
    last: Option<Pose>,
    reached: isize,
}

#[pymethods]
impl ScenePlayback {
    #[new]
    fn new() -> Self {
        Self::default()
    }

    fn is_active(&self) -> bool {
        self.motion.is_some()
    }
    fn is_playing(&self) -> bool {
        matches!(self.motion, Some(Motion::Play(_)))
    }

    fn start_glide(&mut self, from_pose: Pose, to: Pose, seconds: f64, now: f64) -> bool {
        self.cancel();
        if seconds <= 0. || from_pose.close_to(&to, 1e-6) {
            return false;
        }
        self.begin(
            Motion::Glide {
                from: from_pose,
                to,
                duration: seconds,
            },
            from_pose,
            now,
        );
        true
    }

    fn start_run(
        &mut self,
        from_pose: Pose,
        poses: Vec<Pose>,
        transition: f64,
        pause: f64,
        now: f64,
    ) -> bool {
        self.cancel();
        if poses.is_empty() {
            return false;
        }
        self.begin(
            Motion::Play(Run::new(from_pose, poses, transition, pause, true)),
            from_pose,
            now,
        );
        true
    }
    fn cancel(&mut self) {
        self.motion = None;
        self.last = None;
    }
    fn stop(&mut self) -> Option<usize> {
        let arrive = matches!(self.motion, Some(Motion::Glide { .. }));
        self.cancel();
        arrive.then_some(0)
    }
    fn accept_camera(&mut self, pose: Pose) {
        if self.is_active() {
            self.last = Some(pose)
        }
    }
    fn tick(&mut self, now: f64, current: Pose) -> (Option<Pose>, Option<usize>) {
        let Some(motion) = &self.motion else {
            return (None, None);
        };
        if self.last.is_some_and(|p| !p.close_to(&current, 1e-5)) {
            return (None, self.stop());
        }
        let elapsed = now - self.started;
        let (pose, reached) = match motion {
            Motion::Glide { from, to, duration } => {
                let pose = glide(from, to, *duration, elapsed);
                if elapsed >= *duration {
                    self.cancel();
                    return (Some(pose), Some(0));
                }
                (pose, -1)
            }
            Motion::Play(run) => run.at(elapsed),
        };
        self.last = Some(pose);
        let arrival = if reached >= 0 && self.reached != reached {
            self.reached = reached;
            Some(reached as usize)
        } else {
            None
        };
        (Some(pose), arrival)
    }
}
impl ScenePlayback {
    fn begin(&mut self, motion: Motion, current: Pose, now: f64) {
        self.motion = Some(motion);
        self.started = now;
        self.last = Some(current);
        self.reached = -1;
    }
}

/// Playback settings stored on each drawing. Default records serialize to an empty object.
#[pyclass(get_all, set_all, eq, module = "built_core", from_py_object)]
#[derive(Clone, PartialEq)]
pub struct SceneAnimation {
    enabled: bool,
    transition: f64,
    pause: f64,
}

fn seconds(v: &Bound<'_, PyAny>, default: f64) -> PyResult<f64> {
    let py = v.py();
    match py
        .import("builtins")?
        .call_method1("float", (v,))
        .and_then(|s| s.extract::<f64>())
    {
        Ok(s) if s.is_finite() => Ok(s.clamp(0., 60.)),
        Ok(_) => Ok(default),
        Err(e)
            if e.is_instance_of::<PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) =>
        {
            Ok(default)
        }
        Err(e) => Err(e),
    }
}
fn transition_time(v: &Bound<'_, PyAny>) -> PyResult<f64> {
    seconds(v, 2.)
}
fn pause_time(v: &Bound<'_, PyAny>) -> PyResult<f64> {
    seconds(v, 1.)
}
fn enabled_flag(v: &Bound<'_, PyAny>) -> PyResult<bool> {
    v.is_truthy()
}

#[pymethods]
impl SceneAnimation {
    #[new]
    #[pyo3(signature=(enabled=true,transition=2.0,pause=1.0))]
    fn new(
        #[pyo3(from_py_with=enabled_flag)] enabled: bool,
        #[pyo3(from_py_with=transition_time)] transition: f64,
        #[pyo3(from_py_with=pause_time)] pause: f64,
    ) -> Self {
        Self {
            enabled,
            transition,
            pause,
        }
    }
    fn glide_seconds(&self) -> f64 {
        if self.enabled {
            self.transition
        } else {
            0.
        }
    }
    fn to_dict<'p>(&self, py: Python<'p>) -> PyResult<Bound<'p, pyo3::types::PyDict>> {
        let out = pyo3::types::PyDict::new(py);
        if !self.enabled {
            out.set_item("enabled", false)?;
        }
        if self.transition != 2. {
            out.set_item("transition", self.transition)?;
        }
        if self.pause != 1. {
            out.set_item("pause", self.pause)?;
        }
        Ok(out)
    }
    #[staticmethod]
    fn from_dict(raw: &Bound<'_, PyAny>) -> PyResult<Self> {
        let Ok(d) = raw.cast::<pyo3::types::PyDict>() else {
            return Ok(Self::new(true, 2., 1.));
        };
        Ok(Self::new(
            d.get_item("enabled")?
                .as_ref()
                .map_or(Ok(true), enabled_flag)?,
            d.get_item("transition")?
                .as_ref()
                .map_or(Ok(2.), transition_time)?,
            d.get_item("pause")?.as_ref().map_or(Ok(1.), pause_time)?,
        ))
    }
    fn __reduce__<'p>(&self, py: Python<'p>) -> (Bound<'p, pyo3::types::PyType>, (bool, f64, f64)) {
        (
            py.get_type::<Self>(),
            (self.enabled, self.transition, self.pause),
        )
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SceneAnimation>()?;
    m.add_class::<Pose>()?;
    m.add_class::<Run>()?;
    m.add_class::<ScenePlayback>()?;
    m.add_class::<VideoTimeline>()?;
    m.add_function(wrap_pyfunction!(ease, m)?)?;
    m.add_function(wrap_pyfunction!(turn, m)?)?;
    m.add_function(wrap_pyfunction!(between, m)?)?;
    m.add_function(wrap_pyfunction!(glide, m)?)?;
    m.add_function(wrap_pyfunction!(video_seconds, m)?)?;
    m.add_function(wrap_pyfunction!(video_frame_count, m)?)?;
    m.add_function(wrap_pyfunction!(video_at, m)?)?;
    Ok(())
}
