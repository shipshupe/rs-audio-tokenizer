//! Records a WAV file (roughly 3 seconds long) using the default input device and config.
//!
//! The input data is recorded to "$CARGO_MANIFEST_DIR/recorded.wav".

use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, FromSample, Sample, SampleFormat, SampleRate, SupportedBufferSize, SupportedStreamConfig};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Arc, Mutex};
use std::vec;

const BUFFERTIME: u64 = 2;

#[derive(Parser, Debug)]
#[command(version, about = "CPAL record_wav example", long_about = None)]
struct Opt {
    /// The audio device to use
    #[arg(short, long, default_value_t = String::from("default"))]
    device: String,

    /// Use the JACK host
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ),
        feature = "jack"
    ))]
    #[arg(short, long)]
    #[allow(dead_code)]
    jack: bool,
}

fn main() -> Result<(), anyhow::Error> {
    let opt = Opt::parse();

    // Conditionally compile with jack if the feature is specified.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ),
        feature = "jack"
    ))]
    // Manually check for flags. Can be passed through cargo with -- e.g.
    // cargo run --release --example beep --features jack -- --jack
    let host = if opt.jack {
        cpal::host_from_id(cpal::available_hosts()
            .into_iter()
            .find(|id| *id == cpal::HostId::Jack)
            .expect(
                "make sure --features jack is specified. only works on OSes where jack is available",
            )).expect("jack host unavailable")
    } else {
        cpal::default_host()
    };

    #[cfg(any(
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        )),
        not(feature = "jack")
    ))]
    let host = cpal::default_host();

    // Set up the input device and stream with the default input config.
    let device = if opt.device == "default" {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|x| x.name().map(|y| y == opt.device).unwrap_or(false))
    }
    .expect("failed to find input device");

    println!("Input device: {}", device.name()?);

    //create two paths to alternate between recorded_0 and recorded_1
    let path_0 = format!("/tmp/recorded_0.wav").to_owned();
    let path_1 = format!("/tmp/recorded_1.wav").to_owned();

    //semaphore to alternate between the two paths
    let mut sem = false;

    let file = File::create("/tmp/log.txt").expect("Unable to create file");
    let file = Arc::new(Mutex::new(file));

    loop {
        let path: &str;
        let mut i;
        let paths = vec![path_0.clone(), path_1.clone()];
        i = if sem { 1 } else { 0 };
        sem = !sem;


        //construct input_config
        let config = SupportedStreamConfig::new(
            2,
            SampleRate(16000),
            SupportedBufferSize::Range { min: (0), max: (8192) },
            SampleFormat::I16,
        );

        // The WAV file we're recording to.
        let spec = wav_spec_from_config(&config);
        let writer = hound::WavWriter::create(&paths[i], spec)?;
        let writer = Arc::new(Mutex::new(Some(writer)));

        // Run the input stream on a separate thread.
        let writer_2 = writer.clone();

        let err_fn = move |err| {
            eprintln!("an error occurred on stream: {}", err);
        };

        let stream = device.build_input_stream(
            &config.into(),
            move |data, _: &_| write_input_data::<i16, i16>(data, &writer_2),
            err_fn,
            None,
        )?;
            
        stream.play()?;
        
        // Let recording go for BUFFERTIME seconds.
        std::thread::sleep(std::time::Duration::from_secs(BUFFERTIME));
        drop(stream);
        writer.lock().unwrap().take().unwrap().finalize()?;


        //call curl to send the file to the server in a thread
        let file_clone = Arc::clone(&file);
        let handle = std::thread::spawn(move || {
            let output = std::process::Command::new("curl")
                .arg("--data-binary")
                .arg(format!("@{}", &paths[i]))
                .arg("http://localhost:8009/transcribe")
                .output()
                .expect("failed to execute process");
            println!("{}", String::from_utf8_lossy(&output.stdout));
            //append to a log file
            let mut file = file_clone.lock().unwrap();
            file.write(&output.stdout).expect("Unable to write data");
            file.write('\n'.to_string().as_bytes()).expect("Unable to write data");
            });
        }   
    }

    fn sample_format(format: cpal::SampleFormat) -> hound::SampleFormat {
        if format.is_float() {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        }
    }

    fn wav_spec_from_config(config: &cpal::SupportedStreamConfig) -> hound::WavSpec {
        hound::WavSpec {
            channels: config.channels() as _,
            sample_rate: config.sample_rate().0 as _,
            bits_per_sample: (config.sample_format().sample_size() * 8) as _,
            sample_format: sample_format(config.sample_format()),
        }
    }

    type WavWriterHandle = Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>;

    fn write_input_data<T, U>(input: &[T], writer: &WavWriterHandle)
    where
        T: Sample,
        U: Sample + hound::Sample + FromSample<T>,
    {
        if let Ok(mut guard) = writer.try_lock() {
            if let Some(writer) = guard.as_mut() {
                for &sample in input.iter() {
                    let sample: U = U::from_sample(sample);
                    writer.write_sample(sample).ok();
                }
            }
    }
}
