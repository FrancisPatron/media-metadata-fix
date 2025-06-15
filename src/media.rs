use std::fs;
use std::io::{Read, Write, BufWriter, Cursor};
use exif::{In, Tag, Value, Field, Rational};
use chrono::{DateTime, Utc};
use png::{Decoder, Encoder};

pub fn update_png_metadata(
    input_path: &str,
    output_path: Option<&str>,
    latitude: f64,
    longitude: f64,
    altitude: f64,
    datetime: DateTime<Utc>
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = fs::File::open(input_path)?;
    let mut png_data = Vec::new();
    file.read_to_end(&mut png_data)?;

    let exif_buf = create_exif_data(latitude, longitude, altitude, datetime)?;

    let decoder = Decoder::new(&png_data[..]);
    let mut reader = decoder.read_info()?;

    let out_file = fs::File::create(output_path.unwrap_or(input_path))?;
    let mut w = BufWriter::new(out_file);
    let mut encoder = Encoder::new(&mut w, reader.info().width, reader.info().height);
    encoder.set_color(reader.info().color_type);
    encoder.set_depth(reader.info().bit_depth);

    let mut writer = encoder.write_header()?;

    let chunk_type = png::chunk::ChunkType(*b"eXIf");
    writer.write_chunk(chunk_type, &exif_buf)?;

    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf)?;
    writer.write_image_data(&buf)?;
    writer.finish()?;

    Ok(())
}

pub fn update_jpeg_metadata(
    input_path: &str,
    output_path: Option<&str>,
    latitude: f64,
    longitude: f64,
    altitude: f64,
    datetime: DateTime<Utc>
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = fs::File::open(input_path)?;
    let mut jpeg_data = Vec::new();
    file.read_to_end(&mut jpeg_data)?;

    let exif_buf = create_exif_data(latitude, longitude, altitude, datetime)?;

    if jpeg_data.len() < 2 || jpeg_data[0] != 0xFF || jpeg_data[1] != 0xD8 {
        return Err("Invalid JPEG file".into());
    }

    let mut output_data = Vec::new();
    output_data.extend_from_slice(&jpeg_data[0..2]);

    let mut i = 2;
    let mut exif_inserted = false;

    while i < jpeg_data.len() {
        if i + 1 >= jpeg_data.len() || jpeg_data[i] != 0xFF {
            if !exif_inserted {
                insert_exif(&mut output_data, &exif_buf);
                exif_inserted = true;
            }
            output_data.extend_from_slice(&jpeg_data[i..]);
            break;
        }

        let marker = jpeg_data[i + 1];

        match marker {
            0xE1 => {
                if i + 3 >= jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }
                let length = ((jpeg_data[i + 2] as u16) << 8) | (jpeg_data[i + 3] as u16);
                if i + 2 + length as usize > jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }

                i += 2 + length as usize;
                i += 2 + length as usize;
            },
            0xE0 => {
                if i + 3 >= jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }
                let length = ((jpeg_data[i + 2] as u16) << 8) | (jpeg_data[i + 3] as u16);
                if i + 2 + length as usize > jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }

                output_data.extend_from_slice(&jpeg_data[i..i + 2 + length as usize]);
                i += 2 + length as usize;

                if !exif_inserted {
                    insert_exif(&mut output_data, &exif_buf);
                    exif_inserted = true;
                }
            },
            0xDA => {
                if !exif_inserted {
                    insert_exif(&mut output_data, &exif_buf);
                }
                output_data.extend_from_slice(&jpeg_data[i..]);
                break;
            },
            _ if marker >= 0xE2 && marker <= 0xEF => {
                if i + 3 >= jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }
                let length = ((jpeg_data[i + 2] as u16) << 8) | (jpeg_data[i + 3] as u16);
                if i + 2 + length as usize > jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }

                output_data.extend_from_slice(&jpeg_data[i..i + 2 + length as usize]);
                i += 2 + length as usize;
            },
            _ if (marker >= 0xC0 && marker <= 0xFE) && marker != 0xD8 && marker != 0xD9 => {
                if i + 3 >= jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }
                let length = ((jpeg_data[i + 2] as u16) << 8) | (jpeg_data[i + 3] as u16);
                if i + 2 + length as usize > jpeg_data.len() {
                    output_data.extend_from_slice(&jpeg_data[i..]);
                    break;
                }

                output_data.extend_from_slice(&jpeg_data[i..i + 2 + length as usize]);
                i += 2 + length as usize;
            },
            _ => {
                output_data.push(jpeg_data[i]);
                i += 1;
            }
        }
    }

    let out_file = fs::File::create(output_path.unwrap_or(input_path))?;
    let mut writer = BufWriter::new(out_file);
    writer.write_all(&output_data)?;

    Ok(())
}

fn create_exif_data(
    latitude: f64,
    longitude: f64,
    altitude: f64,
    datetime: DateTime<Utc>
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut writer = exif::experimental::Writer::new();

    let gps_version_field = Field {
        tag: Tag::GPSVersionID,
        ifd_num: In::PRIMARY,
        value: Value::Byte(vec![2, 3, 0, 0]),
    };
    writer.push_field(&gps_version_field);

    let lat_deg = latitude.abs().floor();
    let lat_min = (latitude.abs() - lat_deg) * 60.0;
    let lat_min_whole = lat_min.floor();
    let lat_sec = (lat_min - lat_min_whole) * 60.0;

    let lat_ref = if latitude >= 0.0 { "N" } else { "S" };
    let lat_ref_field = Field {
        tag: Tag::GPSLatitudeRef,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![lat_ref.as_bytes().to_vec()]),
    };
    writer.push_field(&lat_ref_field);

    let lat_field = Field {
        tag: Tag::GPSLatitude,
        ifd_num: In::PRIMARY,
        value: Value::Rational(vec![
            Rational { num: lat_deg as u32, denom: 1 },
            Rational { num: lat_min_whole as u32, denom: 1 },
            Rational { num: (lat_sec * 1000000.0) as u32, denom: 1000000 },
        ]),
    };
    writer.push_field(&lat_field);

    let lon_deg = longitude.abs().floor();
    let lon_min = (longitude.abs() - lon_deg) * 60.0;
    let lon_min_whole = lon_min.floor();
    let lon_sec = (lon_min - lon_min_whole) * 60.0;

    let lon_ref = if longitude >= 0.0 { "E" } else { "W" };
    let lon_ref_field = Field {
        tag: Tag::GPSLongitudeRef,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![lon_ref.as_bytes().to_vec()]),
    };
    writer.push_field(&lon_ref_field);

    let lon_field = Field {
        tag: Tag::GPSLongitude,
        ifd_num: In::PRIMARY,
        value: Value::Rational(vec![
            Rational { num: lon_deg as u32, denom: 1 },
            Rational { num: lon_min_whole as u32, denom: 1 },
            Rational { num: (lon_sec * 1000000.0) as u32, denom: 1000000 },
        ]),
    };
    writer.push_field(&lon_field);

    let alt_field = Field {
        tag: Tag::GPSAltitude,
        ifd_num: In::PRIMARY,
        value: Value::Rational(vec![
            Rational { num: (altitude.abs() * 1000.0) as u32, denom: 1000 }
        ]),
    };
    writer.push_field(&alt_field);

    let alt_ref_field = Field {
        tag: Tag::GPSAltitudeRef,
        ifd_num: In::PRIMARY,
        value: Value::Byte(vec![if altitude >= 0.0 { 0 } else { 1 }]),
    };
    writer.push_field(&alt_ref_field);

    let datetime_str = datetime.format("%Y:%m:%d %H:%M:%S").to_string();

    let datetime_field = Field {
        tag: Tag::DateTime,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![datetime_str.as_bytes().to_vec()]),
    };
    writer.push_field(&datetime_field);

    let datetime_orig_field = Field {
        tag: Tag::DateTimeOriginal,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![datetime_str.as_bytes().to_vec()]),
    };
    writer.push_field(&datetime_orig_field);

    let datetime_dig_field = Field {
        tag: Tag::DateTimeDigitized,
        ifd_num: In::PRIMARY,
        value: Value::Ascii(vec![datetime_str.as_bytes().to_vec()]),
    };
    writer.push_field(&datetime_dig_field);

    let mut tiff_buf = Cursor::new(Vec::new());
    writer.write(&mut tiff_buf, false)?;
    let tiff_data = tiff_buf.into_inner();

    let mut buf = Vec::new();
    buf.extend_from_slice(b"Exif\0\0");
    buf.extend_from_slice(&tiff_data);

    Ok(buf)
}

fn insert_exif(output_data: &mut Vec<u8>, exif_buf: &[u8]) {
    output_data.push(0xFF);
    output_data.push(0xE1);
    let length = exif_buf.len() + 2;
    output_data.push((length >> 8) as u8);
    output_data.push(length as u8);
    output_data.extend_from_slice(exif_buf);
}
