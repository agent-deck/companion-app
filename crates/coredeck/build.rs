//! Build script for Core Deck
//!
//! Generates placeholder icons if they don't exist.

use std::fs;
use std::path::Path;

fn main() {
    // Ensure icons directory exists
    let icons_dir = Path::new("assets/icons");
    fs::create_dir_all(icons_dir).expect("Failed to create icons directory");

    // Create placeholder icons if they don't exist
    create_placeholder_icon("assets/icons/tray_connected.png", 0x00, 0xAA, 0x00); // Green
    create_placeholder_icon("assets/icons/tray_disconnected.png", 0x88, 0x88, 0x88); // Grey

    // Tell Cargo to rerun if icons are missing
    println!("cargo:rerun-if-changed=assets/icons/tray_connected.png");
    println!("cargo:rerun-if-changed=assets/icons/tray_disconnected.png");
}

/// Create a minimal 16x16 solid color PNG icon
#[allow(clippy::same_item_push)]
fn create_placeholder_icon(path: &str, r: u8, g: u8, b: u8) {
    let path = Path::new(path);
    if path.exists() {
        return;
    }

    // Create a minimal 16x16 PNG with the given color
    // This is a hand-crafted minimal PNG structure
    let mut png_data = Vec::new();

    // PNG signature
    png_data.extend_from_slice(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);

    // IHDR chunk (image header)
    let ihdr_data = [
        0x00, 0x00, 0x00, 0x10, // Width: 16
        0x00, 0x00, 0x00, 0x10, // Height: 16
        0x08,                   // Bit depth: 8
        0x02,                   // Color type: RGB
        0x00,                   // Compression: deflate
        0x00,                   // Filter: adaptive
        0x00,                   // Interlace: none
    ];
    write_chunk(&mut png_data, b"IHDR", &ihdr_data);

    // IDAT chunk (image data)
    // Create raw image data: filter byte + RGB for each pixel, for each row
    // Each row: 1 filter byte + 16 pixels * 3 bytes = 49 bytes per row
    let row_size = 1 + 16 * 3;
    let mut raw_data = Vec::with_capacity(16 * row_size);
    for _ in 0..16 {
        raw_data.push(0x00); // Filter: none for this row
        for _ in 0..16 {
            raw_data.extend_from_slice(&[r, g, b]);
        }
    }

    // Compress with zlib
    let compressed = compress_zlib(&raw_data);
    write_chunk(&mut png_data, b"IDAT", &compressed);

    // IEND chunk (image end)
    write_chunk(&mut png_data, b"IEND", &[]);

    fs::write(path, &png_data).expect("Failed to write icon file");
    println!("cargo:warning=Created placeholder icon: {}", path.display());
}

fn write_chunk(data: &mut Vec<u8>, chunk_type: &[u8; 4], chunk_data: &[u8]) {
    // Length (4 bytes, big-endian)
    let len = chunk_data.len() as u32;
    data.extend_from_slice(&len.to_be_bytes());

    // Type (4 bytes)
    data.extend_from_slice(chunk_type);

    // Data
    data.extend_from_slice(chunk_data);

    // CRC32 of type + data
    let crc = crc32(chunk_type, chunk_data);
    data.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(chunk_type: &[u8], chunk_data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in chunk_type.iter().chain(chunk_data.iter()) {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn compress_zlib(data: &[u8]) -> Vec<u8> {
    // Minimal zlib compression (store, no compression)
    // This creates valid zlib data but doesn't actually compress
    let mut result = Vec::new();

    // Zlib header (CMF + FLG)
    result.push(0x78); // CMF: deflate with 32K window
    result.push(0x01); // FLG: no dict, fastest compression

    // Deflate blocks
    let mut remaining = data.len();
    let mut offset = 0;

    while remaining > 0 {
        let block_size = remaining.min(65535);
        let is_final = remaining <= 65535;

        // Block header
        result.push(if is_final { 0x01 } else { 0x00 }); // BFINAL + BTYPE (stored)
        result.push((block_size & 0xFF) as u8);
        result.push(((block_size >> 8) & 0xFF) as u8);
        result.push((!block_size & 0xFF) as u8);
        result.push(((!block_size >> 8) & 0xFF) as u8);

        // Block data
        result.extend_from_slice(&data[offset..offset + block_size]);

        offset += block_size;
        remaining -= block_size;
    }

    // Adler-32 checksum
    let checksum = adler32(data);
    result.extend_from_slice(&checksum.to_be_bytes());

    result
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;

    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }

    (b << 16) | a
}
