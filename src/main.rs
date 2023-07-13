use std::error::Error;
use std::io::{BufRead, BufReader, Cursor, ErrorKind, Lines, Read, Write};
use std::mem::swap;
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use embedded_graphics::image::{Image, ImageRaw};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::{
    mono_font::{ascii::*, MonoTextStyle},
    pixelcolor::Rgb888,
    prelude::*,
};
use embedded_graphics_simulator::{
    BinaryColorTheme, OutputSettingsBuilder, SimulatorDisplay, SimulatorEvent,
};
use gdb_protocol::io::GdbServer;
use gdb_protocol::packet::{CheckedPacket, Kind};

const ELF_PATH : &str = "C:/Users/Kate/GitHub/DelugeFirmware/dbt-build-debug-oled/Deluge-debug-oled.elf";

fn find_debug_symbol() -> Result<u32, ErrorKind> {
    use std::process::Command;

    let output = Command::new("arm-none-eabi-nm")
        .arg("-C")
        .arg(ELF_PATH)
        .output()
        .expect("Could not run nm");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    for line in stdout.lines() {
        if line.contains("OLED::oledCurrentImage") {
            let components: Vec<&str> = line.split_whitespace().collect();
            let hex_addr = components[0].trim_start_matches("0x");
            let addr = u32::from_str_radix(hex_addr, 16).unwrap();
            return Ok(addr);
        }
    }
    return Err(ErrorKind::NotFound);
}

fn read_u32(gdb: &mut GdbServer<BufReader<TcpStream>, TcpStream>, addr: u32) -> u32 {
    let request = format!("m{:x},{}", addr, 4);
    gdb.dispatch(&CheckedPacket::from_data(Kind::Packet, request.into()))
        .unwrap();

    match gdb.next_packet().unwrap() {
        Some(p) => {
            let data = p.invalidate_check().data;
            let bytes = hex::decode(data).unwrap();
            let byte_array = [bytes[0], bytes[1], bytes[2], bytes[3]];
            u32::from_le_bytes(byte_array)
        }
        None => 0,
    }
}

fn read_response(gdb: &mut GdbServer<BufReader<TcpStream>, TcpStream>) -> String {
    match gdb.next_packet().unwrap() {
        Some(p) => {
            let data = p.invalidate_check().data;
            let response = std::str::from_utf8(&data).unwrap();
            return response.to_string();
        }
        None => "".to_string(),
    }
}

fn halt(gdb: &mut GdbServer<BufReader<TcpStream>, TcpStream>) {
    gdb.writer.write(&[0x03]).unwrap();
    read_response(gdb);
}

fn cont(gdb: &mut GdbServer<BufReader<TcpStream>, TcpStream>) {
    gdb.dispatch(&CheckedPacket::from_data(Kind::Packet, "c".into()))
        .unwrap();
}

fn main() -> Result<(), core::convert::Infallible> {
    let display_buf_addr = find_debug_symbol().unwrap();
    const DISPLAY_SIZE: Size = Size::new(128, 48);
    let mut display = SimulatorDisplay::<BinaryColor>::new(DISPLAY_SIZE);

    let output_settings = OutputSettingsBuilder::new()
        .theme(BinaryColorTheme::OledWhite)
        .scale(4)
        .pixel_spacing(0)
        .build();
    let mut window =
        embedded_graphics_simulator::Window::new("Deluge OLED output", &output_settings);

    let mut gdb_stream = TcpStream::connect("127.0.0.1:3333").unwrap();
    let mut gdb = GdbServer::new(BufReader::new(gdb_stream.try_clone().unwrap()), gdb_stream);

    const display_buf_size: usize = (DISPLAY_SIZE.width * (DISPLAY_SIZE.height >> 3)) as usize;
    let mut displaybuf = [0u8; 1];

    // Desired frequency in Hz
    let frequency: f64 = 24.0;
    // Duration between iterations in nanoseconds
    let interval = Duration::from_nanos((1_000_000_000f64 / frequency) as u64);

    // Store the start time of the loop
    let mut last_time = Instant::now();

    let mut display_buf = [0; ((DISPLAY_SIZE.width * DISPLAY_SIZE.height) / 8) as usize];

    loop {
        //halt(&mut gdb);

        // Fetch the pointer result of OLED::currentCurrentImage
        let current_image_buf_addr = read_u32(&mut gdb, display_buf_addr);

        let request = format!("m{:x},{:x}", current_image_buf_addr, display_buf_size);
        gdb.dispatch(&CheckedPacket::from_data(Kind::Packet, request.into()))
            .unwrap();

        let decoded = match gdb.next_packet().unwrap() {
            Some(p) => {
                let data = p.invalidate_check().data;
                let bytes = hex::decode(data).unwrap();
                bytes.into_iter().map(|b| u8::from_le(b)).collect()
            }
            None => Vec::new(),
        };

        //cont(&mut gdb);

        for (page_y, row) in decoded.chunks(DISPLAY_SIZE.width as usize).enumerate() {
            for (x, col) in row.into_iter().enumerate() {
                for bit in 0..8 {
                    let y = (page_y * 8) + bit;
                    let buf_idx = ((y * DISPLAY_SIZE.width as usize) + x) / 8;
                    let bitmask = 1u8 << (7 - (x % 8));
                    if (col >> bit) & 0b1 == 1 {
                        display_buf[buf_idx] |= bitmask
                    } else {
                        display_buf[buf_idx] &= !bitmask
                    }
                }
            }
        }
        // println!("{:?}", displaybuf);
        let raw_image = ImageRaw::<BinaryColor>::new(&display_buf, DISPLAY_SIZE.width);
        let image = Image::new(&raw_image, Point::zero());

        image.draw(&mut display)?;

        window.update(&display);
        if window.events().any(|e| e == SimulatorEvent::Quit) {
            break;
        }

        let elapsed = last_time.elapsed();

        // Sleep for the remaining time to achieve the desired frequency
        if elapsed < interval {
            thread::sleep(interval - elapsed);
        }

        // Update the last iteration time
        last_time = Instant::now();
    }

    Ok(())
}
