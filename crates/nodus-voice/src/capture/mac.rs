//! The default microphone on macOS: Core Audio's voice-processing I/O unit
//! (`kAudioUnitSubType_VoiceProcessingIO`), Apple's echo cancellation, noise
//! suppression and automatic gain, as Chromium's own system echo
//! cancellation on macOS uses it. The unit plays silence on its output side
//! (it needs both sides running) and hands 48 kHz mono float to the sink,
//! converting from the device itself.
//!
//! AVAudioEngine's voice processing was tried first: with the output chain
//! present the engine wouldn't start (error -10875), and without it the
//! input tap was never called.

use std::ffi::c_void;
use std::ptr::{NonNull, null_mut};

use objc2_audio_toolbox::{
    AURenderCallbackStruct, AUVoiceIOOtherAudioDuckingConfiguration, AUVoiceIOOtherAudioDuckingLevel,
    AudioComponentDescription, AudioComponentFindNext, AudioComponentInstanceDispose, AudioComponentInstanceNew,
    AudioOutputUnitStart, AudioOutputUnitStop, AudioUnit, AudioUnitInitialize, AudioUnitRender,
    AudioUnitRenderActionFlags, AudioUnitSetProperty, AudioUnitUninitialize,
    kAUVoiceIOProperty_OtherAudioDuckingConfiguration, kAUVoiceIOProperty_VoiceProcessingEnableAGC,
    kAudioOutputUnitProperty_EnableIO, kAudioOutputUnitProperty_SetInputCallback, kAudioUnitManufacturer_Apple,
    kAudioUnitProperty_SetRenderCallback, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Global,
    kAudioUnitScope_Input, kAudioUnitScope_Output, kAudioUnitSubType_VoiceProcessingIO, kAudioUnitType_Output,
};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp, kAudioFormatFlagIsFloat,
    kAudioFormatFlagIsPacked, kAudioFormatLinearPCM,
};

use super::DeviceThread;
use crate::recorder::{OpenError, Sink, Source};
use crate::resample::RATE;

/// The unit's input element (the microphone) and output element (the speaker).
const INPUT: u32 = 1;
const OUTPUT: u32 = 0;
/// Frames the input callback's buffer holds before it has to grow.
const BUFFER_FRAMES: usize = 8192;

pub(super) fn open(sink: Sink) -> Result<Box<dyn Source>, OpenError> {
    let device = DeviceThread::spawn("nodus-voice-mic", move |ready, stop| {
        let unit = match Unit::start(sink) {
            Ok(unit) => {
                let _ = ready.send(Ok(RATE));
                unit
            }
            Err(err) => {
                let _ = ready.send(Err(err));
                return;
            }
        };
        // Captures until the recording stops or is cancelled.
        let _ = stop.recv();
        drop(unit);
    })?;
    Ok(Box::new(device))
}

/// What the input callback needs.
struct Capture {
    unit: AudioUnit,
    sink: Sink,
    buffer: Vec<f32>,
}

/// A started voice-processing unit; dropping it stops and disposes of it.
struct Unit {
    unit: AudioUnit,
    capture: *mut Capture,
}

/// 48 kHz mono 32-bit float, packed.
fn format() -> AudioStreamBasicDescription {
    AudioStreamBasicDescription {
        mSampleRate: f64::from(RATE),
        mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
        mBytesPerPacket: 4,
        mFramesPerPacket: 1,
        mBytesPerFrame: 4,
        mChannelsPerFrame: 1,
        mBitsPerChannel: 32,
        mReserved: 0,
    }
}

/// Sets a property from a value.
///
/// # Safety
///
/// `unit` must be a live audio unit and `T` the property's type.
unsafe fn set<T>(unit: AudioUnit, property: u32, scope: u32, element: u32, value: &T) -> Result<(), OpenError> {
    // SAFETY: the caller vouches for the unit and the type; the value lives across the call.
    let status = unsafe {
        AudioUnitSetProperty(
            unit,
            property,
            scope,
            element,
            std::ptr::from_ref(value).cast::<c_void>(),
            size_of::<T>() as u32,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(OpenError::NotReadable)
    }
}

impl Unit {
    fn start(sink: Sink) -> Result<Self, OpenError> {
        let desc = AudioComponentDescription {
            componentType: kAudioUnitType_Output,
            componentSubType: kAudioUnitSubType_VoiceProcessingIO,
            componentManufacturer: kAudioUnitManufacturer_Apple,
            componentFlags: 0,
            componentFlagsMask: 0,
        };
        // SAFETY: Core Audio calls on a unit this function creates; the
        // callback state is boxed and outlives the unit (freed in Drop after
        // the unit is disposed of).
        unsafe {
            let component = AudioComponentFindNext(null_mut(), NonNull::from(&desc));
            if component.is_null() {
                return Err(OpenError::NotReadable);
            }
            let mut unit: AudioUnit = null_mut();
            if AudioComponentInstanceNew(component, NonNull::from(&mut unit)) != 0 || unit.is_null() {
                return Err(OpenError::NotReadable);
            }
            let capture = Box::into_raw(Box::new(Capture {
                unit,
                sink,
                buffer: vec![0.0; BUFFER_FRAMES],
            }));
            let this = Self { unit, capture };
            let on: u32 = 1;
            set(
                unit,
                kAudioOutputUnitProperty_EnableIO,
                kAudioUnitScope_Input,
                INPUT,
                &on,
            )?;
            set(
                unit,
                kAudioUnitProperty_StreamFormat,
                kAudioUnitScope_Output,
                INPUT,
                &format(),
            )?;
            set(
                unit,
                kAudioUnitProperty_StreamFormat,
                kAudioUnitScope_Input,
                OUTPUT,
                &format(),
            )?;
            let input = AURenderCallbackStruct {
                inputProc: Some(on_input),
                inputProcRefCon: capture.cast(),
            };
            set(
                unit,
                kAudioOutputUnitProperty_SetInputCallback,
                kAudioUnitScope_Global,
                OUTPUT,
                &input,
            )?;
            let silence = AURenderCallbackStruct {
                inputProc: Some(on_output),
                inputProcRefCon: null_mut(),
            };
            set(
                unit,
                kAudioUnitProperty_SetRenderCallback,
                kAudioUnitScope_Input,
                OUTPUT,
                &silence,
            )?;
            // Automatic gain, as getUserMedia's autoGainControl.
            set(
                unit,
                kAUVoiceIOProperty_VoiceProcessingEnableAGC,
                kAudioUnitScope_Global,
                INPUT,
                &on,
            )?;
            // Chromium doesn't duck other apps' audio; duck it as little as
            // macOS 14 and later allow (older systems refuse the property).
            let ducking = AUVoiceIOOtherAudioDuckingConfiguration {
                mEnableAdvancedDucking: 0,
                mDuckingLevel: AUVoiceIOOtherAudioDuckingLevel::Min,
            };
            let _ = set(
                unit,
                kAUVoiceIOProperty_OtherAudioDuckingConfiguration,
                kAudioUnitScope_Global,
                OUTPUT,
                &ducking,
            );
            if AudioUnitInitialize(unit) != 0 {
                return Err(OpenError::NotReadable);
            }
            if AudioOutputUnitStart(unit) != 0 {
                AudioUnitUninitialize(unit);
                return Err(OpenError::NotReadable);
            }
            Ok(this)
        }
    }
}

impl Drop for Unit {
    fn drop(&mut self) {
        // SAFETY: stops and disposes of the unit this owns; once disposed of,
        // no callback can run, so the callback state can be freed.
        unsafe {
            AudioOutputUnitStop(self.unit);
            AudioUnitUninitialize(self.unit);
            AudioComponentInstanceDispose(self.unit);
            drop(Box::from_raw(self.capture));
        }
    }
}

/// The microphone has `frames` ready: render them into our buffer and pass them on.
unsafe extern "C-unwind" fn on_input(
    ref_con: NonNull<c_void>,
    flags: NonNull<AudioUnitRenderActionFlags>,
    time: NonNull<AudioTimeStamp>,
    bus: u32,
    frames: u32,
    _data: *mut AudioBufferList,
) -> i32 {
    // SAFETY: `ref_con` is the boxed `Capture` the unit was given, which
    // outlives it; Core Audio calls this on one thread at a time.
    let capture = unsafe { ref_con.cast::<Capture>().as_mut() };
    let count = frames as usize;
    if capture.buffer.len() < count {
        capture.buffer.resize(count, 0.0);
    }
    let mut list = AudioBufferList {
        mNumberBuffers: 1,
        mBuffers: [AudioBuffer {
            mNumberChannels: 1,
            mDataByteSize: frames * 4,
            mData: capture.buffer.as_mut_ptr().cast(),
        }],
    };
    // SAFETY: renders `frames` mono floats into a buffer that holds them.
    let status = unsafe {
        AudioUnitRender(
            capture.unit,
            flags.as_ptr(),
            time,
            bus,
            frames,
            NonNull::from(&mut list),
        )
    };
    if status == 0 {
        capture.sink.push(&capture.buffer[..count]);
    }
    status
}

/// The speaker side plays nothing.
unsafe extern "C-unwind" fn on_output(
    _ref_con: NonNull<c_void>,
    flags: NonNull<AudioUnitRenderActionFlags>,
    _time: NonNull<AudioTimeStamp>,
    _bus: u32,
    _frames: u32,
    data: *mut AudioBufferList,
) -> i32 {
    // SAFETY: Core Audio hands a valid buffer list for the call; each
    // buffer's data holds `mDataByteSize` bytes.
    unsafe {
        if let Some(list) = data.as_mut() {
            let buffers = std::slice::from_raw_parts_mut(list.mBuffers.as_mut_ptr(), list.mNumberBuffers as usize);
            for buffer in buffers {
                if !buffer.mData.is_null() {
                    std::ptr::write_bytes(buffer.mData.cast::<u8>(), 0, buffer.mDataByteSize as usize);
                }
            }
        }
        let flags = flags.as_ptr();
        (*flags).0 |= AudioUnitRenderActionFlags::UnitRenderAction_OutputIsSilence.0;
    }
    0
}
