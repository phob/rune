//! The frame producer: a real D3D11 device on Rune's adapter writes the scRGB pattern into FP16
//! textures shared by name (`D3D11_RESOURCE_MISC_SHARED | D3D11_RESOURCE_MISC_SHARED_NTHANDLE`),
//! signals a shared `ID3D11Fence` once each is written and waits for Rune's release fence before
//! writing one again: what libmpv's `gpu-api=d3d11` exporter does with its frames.

#![allow(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rune::NativeSurface;
use rune::native_surface::windows::{SharedFormat, SharedTexture, adapter_luid};
use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_FENCE_FLAG_SHARED,
    D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11CreateDevice, ID3D11Device, ID3D11Device5, ID3D11DeviceContext, ID3D11DeviceContext4, ID3D11Fence, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_DESC1, IDXGIAdapter1, IDXGIFactory4, IDXGIResource1};
use windows::core::{HSTRING, Interface};

use crate::pattern::{self, HEIGHT, WIDTH};

/// Textures the producer cycles through, as a video renderer's swap ring does.
const RING: usize = 3;

/// What the producer did, for the report.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub adapter: String,
    /// `DXGI_ADAPTER_DESC1::VendorId` of Rune's adapter (0x10DE is NVIDIA).
    pub vendor_id: u32,
    pub luid: (i32, u32),
    pub texture_format: &'static str,
    pub textures: Vec<String>,
    pub ready_fence: String,
    pub release_fence: String,
    pub frames_pushed: u64,
    /// The level the pattern's SDR-white patch was written at: the window's SDR white when the
    /// producer started (a window switched to HDR later keeps that picture).
    pub pattern_sdr_white: f32,
    /// The release fence's completed value once Rune read the first frame: Rune's queue signalled it.
    pub release_completed: u64,
    pub error: Option<String>,
}

struct Shared {
    texture: ID3D11Texture2D,
    handle: HANDLE,
    name: String,
    /// The fence value of the frame last written into it.
    value: u64,
}

struct Producer {
    _device: ID3D11Device,
    context: ID3D11DeviceContext4,
    ring: Vec<Shared>,
    ready: (ID3D11Fence, HANDLE),
    release: (ID3D11Fence, HANDLE),
}

impl Drop for Producer {
    fn drop(&mut self) {
        // SAFETY: the handles this producer opened, each closed once.
        unsafe {
            for s in &self.ring { CloseHandle(s.handle).ok(); }
            CloseHandle(self.ready.1).ok();
            CloseHandle(self.release.1).ok();
        }
    }
}

/// Starts the producer's thread: once Rune has made its device and `start` says the level SDR
/// white is drawn at, it pushes the pattern into `surface` (once when `animate` is false, else
/// with a moving 1000-nit bar at 30 fps).
pub fn spawn(surface: NativeSurface, start: std::sync::mpsc::Receiver<f32>, animate: bool, stats: Arc<Mutex<Stats>>) {
    std::thread::Builder::new().name("hdr-producer".into()).spawn(move || {
        if let Err(error) = run(&surface, &start, animate, &stats) {
            eprintln!("hdr_verify: producer: {error}");
            stats.lock().unwrap().error = Some(error);
        }
    }).expect("the producer thread starts");
}

fn run(surface: &NativeSurface, start: &std::sync::mpsc::Receiver<f32>, animate: bool, stats: &Mutex<Stats>) -> Result<(), String> {
    let sdr_white = start.recv_timeout(Duration::from_secs(30)).map_err(|_| "the window never drew its first frame")?;
    let luid = adapter_luid().ok_or("Rune made no D3D12 window device")?;
    let base = format!("rune-hdr-verify-{}", std::process::id());
    let mut producer = Producer::new(luid, &base, stats)?;
    let texels = pattern::texels(sdr_white);
    stats.lock().unwrap().pattern_sdr_white = sdr_white;
    for value in 1u64.. {
        let slot = (value as usize - 1) % RING;
        producer.wait_released(slot)?;
        let frame = if animate { with_bar(&texels, value) } else { texels.clone() };
        producer.write(slot, &frame, value)?;
        let s = &producer.ring[slot];
        let shared = SharedTexture::named(&s.name, WIDTH, HEIGHT)
            .format(SharedFormat::Rgba16Float)
            .ready_fence(format!("{base}-written"), value)
            .release_fence(format!("{base}-read"), value);
        surface.push_shared(shared, value)?;
        stats.lock().unwrap().frames_pushed = value;
        if !animate {
            // Rune signals the release fence on its queue once it has copied the frame.
            let released = producer.await_release(value, Duration::from_secs(20))?;
            stats.lock().unwrap().release_completed = released;
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(33));
    }
    Ok(())
}

/// The pattern with a 1000-nit bar across its bottom strip, at frame `value`'s place.
fn with_bar(texels: &[half::f16], value: u64) -> Vec<half::f16> {
    let mut out = texels.to_vec();
    let x0 = (value * 8 % u64::from(WIDTH)) as u32;
    for y in 416..440 {
        for x in x0..(x0 + 48).min(WIDTH) {
            let i = ((y * WIDTH + x) * 4) as usize;
            out[i..i + 3].fill(half::f16::from_f32(12.5));
        }
    }
    out
}

impl Producer {
    fn new(luid: (i32, u32), base: &str, stats: &Mutex<Stats>) -> Result<Producer, String> {
        let err = |what: &str| { let what = what.to_owned(); move |e: windows::core::Error| format!("{what}: {}", e.message()) };
        // SAFETY: DXGI and D3D11 calls with live interfaces and out-pointers this function owns;
        // every handle is kept by the producer and closed when it drops.
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory1().map_err(err("DXGI factory"))?;
            let adapter: IDXGIAdapter1 = factory.EnumAdapterByLuid(LUID { LowPart: luid.1, HighPart: luid.0 }).map_err(err("Rune's adapter"))?;
            let desc: DXGI_ADAPTER_DESC1 = adapter.GetDesc1().map_err(err("adapter description"))?;
            let name_end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1]), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut context)).map_err(err("D3D11 device"))?;
            let device: ID3D11Device = device.ok_or("no D3D11 device")?;
            let context: ID3D11DeviceContext = context.ok_or("no D3D11 context")?;
            let device5: ID3D11Device5 = device.cast().map_err(err("ID3D11Device5"))?;
            let context: ID3D11DeviceContext4 = context.cast().map_err(err("ID3D11DeviceContext4"))?;
            let texture_desc = D3D11_TEXTURE2D_DESC {
                Width: WIDTH,
                Height: HEIGHT,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
            };
            let mut ring = Vec::with_capacity(RING);
            for i in 0..RING {
                let mut texture = None;
                device.CreateTexture2D(&texture_desc, None, Some(&mut texture)).map_err(err("FP16 shared texture"))?;
                let texture: ID3D11Texture2D = texture.ok_or("no texture")?;
                let name = format!("{base}-frame-{i}");
                let handle = texture.cast::<IDXGIResource1>().map_err(err("IDXGIResource1"))?
                    .CreateSharedHandle(None, GENERIC_ALL.0, &HSTRING::from(&name)).map_err(err("share texture"))?;
                ring.push(Shared { texture, handle, name, value: 0 });
            }
            let fence = |name: String| -> Result<(ID3D11Fence, HANDLE), String> {
                let mut fence: Option<ID3D11Fence> = None;
                device5.CreateFence(0, D3D11_FENCE_FLAG_SHARED, &mut fence).map_err(err("shared fence"))?;
                let fence = fence.ok_or("no fence")?;
                let handle = fence.CreateSharedHandle(None, GENERIC_ALL.0, &HSTRING::from(&name)).map_err(err("share fence"))?;
                Ok((fence, handle))
            };
            let ready = fence(format!("{base}-written"))?;
            let release = fence(format!("{base}-read"))?;
            *stats.lock().unwrap() = Stats {
                adapter: String::from_utf16_lossy(&desc.Description[..name_end]),
                vendor_id: desc.VendorId,
                luid,
                texture_format: "DXGI_FORMAT_R16G16B16A16_FLOAT",
                textures: ring.iter().map(|s| s.name.clone()).collect(),
                ready_fence: format!("{base}-written"),
                release_fence: format!("{base}-read"),
                ..Stats::default()
            };
            Ok(Producer { _device: device, context, ring, ready, release })
        }
    }

    /// Waits for Rune to have read the frame last written into `slot`.
    fn wait_released(&self, slot: usize) -> Result<(), String> {
        let value = self.ring[slot].value;
        if value == 0 { return Ok(()); }
        self.await_release(value, Duration::from_secs(5)).map(|_| ())
    }

    /// Waits for the release fence to reach `value`, then says what it reached.
    fn await_release(&self, value: u64, limit: Duration) -> Result<u64, String> {
        let since = Instant::now();
        loop {
            // SAFETY: a plain query on a live fence.
            let completed = unsafe { self.release.0.GetCompletedValue() };
            if completed >= value { return Ok(completed); }
            if since.elapsed() > limit { return Err(format!("Rune never released frame {value} (release fence at {completed})")); }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Writes `texels` into `slot` on the GPU and signals the ready fence to `value` after it.
    fn write(&mut self, slot: usize, texels: &[half::f16], value: u64) -> Result<(), String> {
        let s = &mut self.ring[slot];
        // SAFETY: `texels` holds `HEIGHT` rows of `WIDTH` RGBA16F texels, which the copy reads; the
        // signal is ordered after it on the immediate context, and the flush sends both.
        unsafe {
            self.context.UpdateSubresource(&s.texture, 0, None, texels.as_ptr().cast(), WIDTH * 8, 0);
            self.context.Signal(&self.ready.0, value).map_err(|e| format!("signal ready: {}", e.message()))?;
            self.context.Flush();
        }
        s.value = value;
        Ok(())
    }
}
