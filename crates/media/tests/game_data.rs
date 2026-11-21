//! Tests against the user's game installation. They skip themselves when `AERUGO_GAME_DIR` is not set.

use std::path::PathBuf;

use aerugo_media::{Asf, StreamKind};
use aerugo_unity::{Archive, ReadOpts};

#[test]
fn every_movie_demuxes_to_its_frame_count() {
    let Some(game) = std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from) else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let dir = game.join("smt3hd_Data/StreamingAssets/PC");
    let mut movies = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !(name.starts_with('m')
            && name[1..].chars().next().is_some_and(|c| c.is_ascii_digit() || c == 't' || c == 'e'))
        {
            continue;
        }
        let Ok(ar) = Archive::open(&path) else { continue };
        for (asset, r) in &ar.container {
            if !asset.ends_with(".wmv") {
                continue;
            }
            let (class, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
            assert_eq!(class, "VideoClip");
            let res = &v["m_ExternalResources"];
            let data = ar
                .stream(
                    res["m_Source"].as_str().unwrap(),
                    res["m_Offset"].as_u64().unwrap(),
                    res["m_Size"].as_u64().unwrap(),
                )
                .unwrap();
            let asf = Asf::parse(data).unwrap_or_else(|e| panic!("{name} {asset}: {e:#}"));
            let objects = asf.objects().unwrap_or_else(|e| panic!("{name} {asset}: {e:#}"));
            let video = objects.iter().filter(|o| asf.kind(o.stream) == Some(StreamKind::Video)).count();
            // The clip's frame count comes from its duration times its frame rate, so it can exceed the stored
            // frames by one (e617: 301 frames over 10.068 s).
            let stated = v["m_FrameCount"].as_u64().unwrap();
            assert!(stated.abs_diff(video as u64) <= 1, "{name} {asset}: {video} frames, clip says {stated}");
            eprintln!("{name} {asset}: {video} frames (clip {stated}), {} objects", objects.len());
            assert!(
                objects.iter().any(|o| asf.kind(o.stream) == Some(StreamKind::Audio)),
                "{name} {asset}: no audio"
            );
            movies += 1;
        }
    }
    assert!(movies >= 30, "only {movies} movies found");
}

#[test]
#[ignore]
fn print_audio_formats() {
    let game = PathBuf::from(std::env::var_os("AERUGO_GAME_DIR").unwrap());
    let ar = Archive::open(&game.join("smt3hd_Data/StreamingAssets/PC/m616")).unwrap();
    let (_, r) = ar.container.iter().find(|(a, _)| a.ends_with(".wmv")).unwrap();
    let (_, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
    let res = &v["m_ExternalResources"];
    let data = ar
        .stream(
            res["m_Source"].as_str().unwrap(),
            res["m_Offset"].as_u64().unwrap(),
            res["m_Size"].as_u64().unwrap(),
        )
        .unwrap();
    let asf = Asf::parse(data).unwrap();
    println!("{:?}", asf.audio);
    println!("{:?} {:?}", asf.video[0].1.fourcc, asf.video[0].1.extra);
    // Every movie's video format.
    let dir = game.join("smt3hd_Data/StreamingAssets/PC");
    let mut formats = std::collections::BTreeMap::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.starts_with('m') {
            continue;
        }
        let Ok(ar) = Archive::open(&path) else { continue };
        for (asset, r) in ar.container.iter().filter(|(a, _)| a.ends_with(".wmv")) {
            let (_, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
            let res = &v["m_ExternalResources"];
            let data = ar
                .stream(
                    res["m_Source"].as_str().unwrap(),
                    res["m_Offset"].as_u64().unwrap(),
                    res["m_Size"].as_u64().unwrap(),
                )
                .unwrap();
            let asf = Asf::parse(data).unwrap();
            let f = &asf.video[0].1;
            let first: Vec<u8> = asf
                .objects()
                .unwrap()
                .iter()
                .filter(|o| asf.kind(o.stream) == Some(StreamKind::Video))
                .take(1)
                .flat_map(|o| o.data[..2].to_vec())
                .collect();
            formats
                .entry((f.fourcc, f.width, f.height, f.extra.clone()))
                .or_insert_with(Vec::new)
                .push((asset.clone(), first));
        }
    }
    for (k, v) in formats {
        println!("{k:?}: {} movies, first {:?}", v.len(), v[0]);
    }
}

/// Decode one movie's audio to raw f32 (for comparison with a reference decoder): set AERUGO_DUMP=<bundle>:<out>.
#[test]
#[ignore]
fn dump_movie_audio() {
    let game = PathBuf::from(std::env::var_os("AERUGO_GAME_DIR").unwrap());
    let spec = std::env::var("AERUGO_DUMP").unwrap();
    let (bundle, out) = spec.split_once(':').unwrap();
    let ar = Archive::open(&game.join("smt3hd_Data/StreamingAssets/PC").join(bundle)).unwrap();
    let (_, r) = ar.container.iter().find(|(a, _)| a.ends_with(".wmv")).unwrap();
    let (_, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
    let res = &v["m_ExternalResources"];
    let data = ar
        .stream(
            res["m_Source"].as_str().unwrap(),
            res["m_Offset"].as_u64().unwrap(),
            res["m_Size"].as_u64().unwrap(),
        )
        .unwrap();
    let asf = Asf::parse(data).unwrap();
    let (num, fmt) = &asf.audio[0];
    let mut dec = aerugo_media::WmaDecoder::new(fmt).unwrap();
    let mut pcm = Vec::new();
    let objects: Vec<_> = asf.objects().unwrap().into_iter().filter(|o| o.stream == *num).collect();
    let mut sizes = std::collections::BTreeMap::new();
    for o in &objects {
        *sizes.entry(o.data.len()).or_insert(0) += 1;
    }
    println!("audio object sizes: {sizes:?}");
    for (i, o) in objects.iter().enumerate() {
        for (j, sf) in o.data.chunks(fmt.block_align as usize).enumerate() {
            match dec.decode(sf) {
                Ok(p) => pcm.extend(p),
                Err(e) => panic!("object {i} superframe {j}: {e:#}"),
            }
        }
    }
    let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(out, bytes).unwrap();
    println!("{} frames, {} continuation-only superframes", pcm.len() / 2, dec.continuation_only);
}

#[test]
fn every_movie_audio_decodes_to_its_duration() {
    let Some(game) = std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from) else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let dir = game.join("smt3hd_Data/StreamingAssets/PC");
    // One of each distinct movie: the language variants and alternate cuts share their encoding.
    for bundle in [
        "m616",
        "m617",
        "m658",
        "m673",
        "m674",
        "m676",
        "m684",
        "m686",
        "m727",
        "m731",
        "m804",
        "mevent",
        "mtitle",
        "mtitle_en",
    ] {
        let ar = Archive::open(&dir.join(bundle)).unwrap();
        for (asset, r) in ar.container.iter().filter(|(a, _)| a.ends_with(".wmv")) {
            let (_, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
            let res = &v["m_ExternalResources"];
            let data = ar
                .stream(
                    res["m_Source"].as_str().unwrap(),
                    res["m_Offset"].as_u64().unwrap(),
                    res["m_Size"].as_u64().unwrap(),
                )
                .unwrap();
            let asf = Asf::parse(data).unwrap();
            let (num, fmt) = &asf.audio[0];
            let mut dec = aerugo_media::WmaDecoder::new(fmt).unwrap();
            let mut frames = 0;
            for o in asf.objects().unwrap().iter().filter(|o| o.stream == *num) {
                for sf in o.data.chunks(fmt.block_align as usize) {
                    frames += dec.decode(sf).unwrap_or_else(|e| panic!("{bundle} {asset}: {e:#}")).len() / 2;
                }
            }
            let secs = (frames - dec.delay()) as f64 / 48000.0;
            let play = asf.play_duration_100ns as f64 / 1e7 - asf.preroll_ms as f64 / 1000.0;
            assert!(
                (secs - play).abs() < 0.2,
                "{bundle} {asset}: {secs:.3} s of audio, file plays {play:.3} s"
            );
        }
    }
}

/// Decode one movie's video to raw 4:2:0 (display size, for comparison with a reference decoder):
/// AERUGO_DUMP=<bundle>:<out>[:<frames>]. With `<out>` = `crc`, prints each frame's Adler-32 instead.
#[test]
#[ignore]
fn dump_movie_video() {
    let game = PathBuf::from(std::env::var_os("AERUGO_GAME_DIR").unwrap());
    let spec = std::env::var("AERUGO_DUMP").unwrap();
    let mut parts = spec.split(':');
    let (bundle, out) = (parts.next().unwrap(), parts.next().unwrap());
    let limit: usize = parts.next().map(|n| n.parse().unwrap()).unwrap_or(usize::MAX);
    let ar = Archive::open(&game.join("smt3hd_Data/StreamingAssets/PC").join(bundle)).unwrap();
    let (_, r) = ar.container.iter().find(|(a, _)| a.ends_with(".wmv")).unwrap();
    let (_, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
    let res = &v["m_ExternalResources"];
    let data = ar
        .stream(
            res["m_Source"].as_str().unwrap(),
            res["m_Offset"].as_u64().unwrap(),
            res["m_Size"].as_u64().unwrap(),
        )
        .unwrap();
    let asf = Asf::parse(data).unwrap();
    let (num, fmt) = &asf.video[0];
    let mut dec = aerugo_media::Vc1Decoder::new(fmt).unwrap();
    let crc_only = out == "crc";
    let mut out_bytes = Vec::new();
    let start = std::time::Instant::now();
    let mut frames = 0;
    for (i, o) in asf.objects().unwrap().iter().filter(|o| o.stream == *num).take(limit).enumerate() {
        let pic = dec.decode(&o.data).unwrap_or_else(|e| panic!("frame {i}: {e:#}"));
        let (w, h) = (pic.display_width, pic.display_height);
        for (p, pw, ph) in [(&pic.y, w, h), (&pic.cb, w / 2, h / 2), (&pic.cr, w / 2, h / 2)] {
            for y in 0..ph {
                out_bytes.extend_from_slice(&p.data[y * p.width..y * p.width + pw]);
            }
        }
        frames += 1;
        if crc_only {
            let (mut a, mut b) = (0u32, 0u32); // the reference tool starts from 0
            for &v in &out_bytes {
                a = (a + v as u32) % 65521;
                b = (b + a) % 65521;
            }
            println!("crc {i} 0x{:08x}", (b << 16) | a);
            out_bytes.clear();
        }
    }
    println!("{frames} frames in {:.2?}", start.elapsed());
    if !crc_only {
        std::fs::write(out, out_bytes).unwrap();
    }
}

#[test]
fn every_movie_starts_decoding() {
    let Some(game) = std::env::var_os("AERUGO_GAME_DIR").map(PathBuf::from) else {
        eprintln!("AERUGO_GAME_DIR not set, skipping");
        return;
    };
    let dir = game.join("smt3hd_Data/StreamingAssets/PC");
    let mut movies = 0;
    for e in std::fs::read_dir(&dir).unwrap() {
        let path = e.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if !name.starts_with('m') {
            continue;
        }
        let Ok(ar) = Archive::open(&path) else { continue };
        for (asset, r) in ar.container.iter().filter(|(a, _)| a.ends_with(".wmv")) {
            let (_, v, _) = ar.read(*r, ReadOpts::default()).unwrap();
            let res = &v["m_ExternalResources"];
            let data = ar
                .stream(
                    res["m_Source"].as_str().unwrap(),
                    res["m_Offset"].as_u64().unwrap(),
                    res["m_Size"].as_u64().unwrap(),
                )
                .unwrap();
            let asf = Asf::parse(data).unwrap();
            let (num, fmt) = &asf.video[0];
            let mut dec =
                aerugo_media::Vc1Decoder::new(fmt).unwrap_or_else(|e| panic!("{name} {asset}: {e:#}"));
            // The first two seconds: the key frame and the predicted frames after it.
            for (i, o) in asf.objects().unwrap().iter().filter(|o| o.stream == *num).take(60).enumerate() {
                let pic = dec.decode(&o.data).unwrap_or_else(|e| panic!("{name} {asset} frame {i}: {e:#}"));
                assert_eq!(
                    (pic.display_width, pic.display_height),
                    (fmt.width as usize, fmt.height as usize)
                );
            }
            movies += 1;
        }
    }
    assert!(movies >= 30, "only {movies} movies found");
}
