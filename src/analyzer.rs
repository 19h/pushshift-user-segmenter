use std::collections::HashMap;
use std::fs::{DirEntry, File};
use std::hash::BuildHasherDefault;
use std::io::{BufReader, Read, Write};
use std::ops::{AddAssign, Div};
use std::ops::Mul;
use std::path::Path;

use cortical_io::density::{Density, Kde};
use num::complex::ComplexFloat;
use num::Float;
use num_traits::FromPrimitive;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelIterator;
use twox_hash::XxHash;
use zstd::Decoder;

use serializer::deserialize;
use text::text_item::TextItem;
use crate::serializer::SerializerFeedback;

use crate::text::STOPWORDS;
use crate::text::text_item::{PooMap, PooMapInner};

mod text;
mod serializer;

fn std_deviation(values: &[f32]) -> f32 {
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / values.len() as f32;
    variance.sqrt()
}

// double y;
//
//     c = c & 0xFF;
//     y = (double) c;
//     y = y / 255.0;
//     if (c <= 0.04045)
//         y = y / 12.92;
//     else
//         y = pow(((y + 0.055) / 1.055), 2.4);
//     return (y);

fn srgb_to_linear(c: f32) -> f32 {
    let y = c / 255.0;
    if y <= 0.04045 {
        y / 12.92
    } else {
        ((y + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn save_fingerpint(poo_map: &PooMapInner, name: &str, fp_type: &str) {
    let mut gwf = {
        let mut f =
            poo_map
                .iter()
                .collect::<Vec<_>>();

        f.sort_by(|a, b| b.1.cmp(a.1));

        let mut p = HashMap::<Vec<u8>, u64, BuildHasherDefault<XxHash>>::default();

        for (k, v) in f.iter().take(128 * 128).cloned() {
            p.insert(k.clone(), *v);
        }

        p
    };

    let mut f =
        gwf
            .iter()
            .map(|(k, v)| *v as u32)
            .collect::<Vec<_>>();

    let f_stddev = std_deviation(&f.iter().map(|x| *x as f32).collect::<Vec<_>>());
    let f_mean = f.iter().sum::<u32>() as f32 / f.len() as f32;

    let f_min = f_mean / 2.0;
    let f_max = f_mean * 2.0;

    println!("f_min: {}, f_max: {}", f_min, f_max);
    println!("f_mean: {}, f_stddev: {}", f_mean, f_stddev);

    //let mut f =
    //    f.iter()
    //        //.filter(|b| **b < f_min)
    //        .cloned()
    //        .map(|x| (((x as f32 - f_min) / (f_max - f_min)) * 255.0) as u32)
    //        .collect::<Vec<_>>();

    use rustfft::{FftPlanner, num_complex::Complex};

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(16384);

    let mut buffer = vec![Complex { re: 0.0, im: 0.0 }; 16384];

    for (i, v) in f.iter().enumerate() {
        buffer[i] = Complex { re: *v as f32, im: 1.0 };
    }

    fft.process(&mut buffer);

    cortical_io::image::generate_height_image_from_vec(
        &buffer.iter().map(|x| x.norm() as u32).collect::<Vec<_>>(),
        //&f,
        10,
        |p, i|
            match p {
                0 => [0, 0, 0],
                //_ if densest_points.contains(&i) => [255, 0, 0],
                _ => {
                    // normalize p
                    let r1 = 255.0 / 255.0;
                    let g1 = 20.0 / 255.0;
                    let b1 = 147.0 / 255.0;

                    let r2 = 255.0 / 255.0;
                    let g2 = 0.0 / 255.0;
                    let b2 = 255.0 / 255.0;

                    let r = (((r2 - r1) * p as f32 + r1) * 255.0) as u8;
                    let g = (((g2 - g1) * p as f32 + g1) * 255.0) as u8;
                    let b = (((b2 - b1) * p as f32 + b1) * 255.0) as u8;

                    //[255 - p, (20 + p).max(255), (147 - p).min(0)]
                    [
                        p / 3,
                        p / 2,
                        p,
                    ]
                },
            },
    ).save(&format!("./fps/{}.{}.png", name, fp_type)).unwrap();
}

fn run_for_file(path: &Path) {
    let name = path.file_name().unwrap().to_str().unwrap().to_string();

    println!("name: {}", name);

    let mut file = File::open(path).unwrap();

    let mut decoder =
        Decoder::new(&mut file).unwrap();

    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf).unwrap();
    //file.read_to_end(&mut buf).unwrap();

    let poo =
        deserialize(
            &buf,
            |x| if let SerializerFeedback::Message(m) = x { println!("{}", m) },
        );

    let author_count = poo.len();

    // create a PooMap merging the frequencies of all comments by the same author
    let mut poo_map = PooMapInner::new();

    for (_, ref mut freqs) in poo.iter() {
        for (word, freq) in freqs.iter() {
            if STOPWORDS.contains(word.iter().map(|&b| b as char).collect::<String>().as_str()) {
                continue;
            }

            poo_map
                .entry(word.clone())
                .or_insert(0)
                .add_assign(*freq);
        }
    }

    save_fingerpint(&poo_map, "global", "global");

    use rustfft::{FftPlanner, num_complex::Complex};

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_inverse(16384);

    let mut authors = poo
        .iter()
        .collect::<Vec<_>>();

    authors.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    let mut authors = authors
        .iter()
        .take(100)
        .collect::<Vec<_>>();

    authors
        .par_iter()
        .for_each(|(author, comments)| {
            let mut xy = poo_map.clone();

            xy.iter_mut()
                .for_each(|(_, v)| *v = 0);

            for (word, ref mut freq) in comments.iter() {
                if xy.contains_key(word) {
                    xy.insert(word.clone(), **freq);
                }
            }

            let author = String::from_utf8_lossy(author.iter().filter(|&b| *b != 0).cloned().collect::<Vec<_>>().as_slice()).to_string();

            // count zeros in xy
            let not_zero_count = xy.iter().filter(|(_, v)| **v > 0).count();

            if not_zero_count < 128 {
                return;
            }

            //save_fingerpint(&xy, &author, "norm");
        });
}

fn main() {
    // find folder located at first argument
    let path = std::env::args().nth(1).expect("No path provided");
    let path = std::path::Path::new(&path);

    // find all files in folder
    let files = std::fs::read_dir(path).expect("Could not read directory");

    // filter for files ending with .zst
    let mut files =
        files
            .filter_map(|f| f.ok())
            .filter(|f| {
                f.path()
                    .extension()
                    .map(|ext| ext == "freqs")
                    .unwrap_or(false)
            })
            .collect::<Vec<DirEntry>>();

    files.sort_by(|a, b| a.path().file_name().cmp(&b.path().file_name()));

    files
        .iter()
        .for_each(|f| {
            run_for_file(&f.path());
        });
}
