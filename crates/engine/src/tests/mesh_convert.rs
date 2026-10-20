use super::*;
use solidcraft_doc::{Document, FeatureKind, MeshData};
use solidcraft_geom::Vec3;

fn imported_box() -> Session {
    let body = solidcraft_kernel::box_solid(Vec3::ZERO, Vec3::new(10., 20., 30.)).unwrap();
    let mesh = body.tessellate(0.01).unwrap();
    let mut doc = Document::default();
    doc.add_feature(
        FeatureKind::MeshImport {
            file: "public-safe-box.stl".into(),
            meshes: vec![MeshData {
                name: "Mesh".into(),
                positions: mesh.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect(),
                triangles: mesh.triangles.iter().flatten().copied().collect(),
                color: Some([0.2, 0.5, 0.8]),
            }],
        },
        Some("Imported mesh"),
    )
    .unwrap();
    Session::new(doc)
}

#[test]
fn mesh_conversion_is_parametric_undoable_and_editable_after_cold_reopen() {
    let mut s = imported_box();
    run(&mut s, "parameters.add", json!({"name":"size","expression":"1","unit":""}));
    run(&mut s, "solid.scale", json!({"bodies":["Mesh"],"factor":"size","name":"Mesh scale"}));
    run(&mut s, "mesh.convert", json!({"body":"Mesh","tolerance":"0.01 mm","name":"Reconstructed prism"}));
    let check = |s: &Session| {
        let state = s.model.state();
        let body = &state.body("Mesh").unwrap().body;
        assert!(!body.is_mesh());
        assert_eq!(body.faces(0.01).unwrap().len(), 6);
        assert_eq!(body.color(), Some([0.2, 0.5, 0.8]));
        assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results);
    };
    check(&s);
    assert!((volume(&mut s) - 6000.).abs() < 1e-6);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.model.state().body("Mesh").unwrap().body.is_mesh());
    run(&mut s, "edit.redo", json!({}));
    check(&s);
    run(&mut s, "parameters.change", json!({"name":"size","expression":"2"}));
    check(&s);
    assert!((volume(&mut s) - 48000.).abs() < 1e-5);
    let mut cold = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
    check(&cold);
    assert!((volume(&mut cold) - 48000.).abs() < 1e-5);
    run(&mut cold, "solid.fillet", json!({"body":"Mesh","edges":[[0,20,60]],"radius":2}));
    assert!(volume(&mut cold) < 48000.);
    assert!(cold.model.results.iter().all(|r| r.error.is_none()));
}

#[test]
fn failed_mesh_conversions_leave_the_design_unchanged() {
    let mut s = imported_box();
    let before = s.doc.to_json();
    for params in [
        json!({}),
        json!({"body":"Missing"}),
        json!({"body":"Mesh","tolerance":0}),
        json!({"body":"Mesh","tolerance":"unknown"}),
        json!({"body":"Mesh","tolerance":"2 mm"}),
    ] {
        assert!(s.execute("mesh.convert", &params).is_err());
        assert_eq!(s.doc.to_json(), before);
        assert!(s.model.state().body("Mesh").unwrap().body.is_mesh());
    }
}

#[test]
fn convert_uses_the_selected_mesh_body_when_body_is_omitted() {
    let mut s = imported_box();
    s.selection = vec![crate::Sel::Body { name: "Mesh".into() }];
    run(&mut s, "mesh.convert", json!({"tolerance":"0.01 mm"}));
    let state = s.model.state();
    let body = &state.body("Mesh").unwrap().body;
    assert!(!body.is_mesh());
    assert_eq!(body.faces(0.01).unwrap().len(), 6);
}

#[test]
fn convert_without_body_or_selection_is_refused() {
    let mut s = imported_box();
    let before = s.doc.to_json();
    assert!(s.execute("mesh.convert", &json!({"tolerance":"0.01 mm"})).is_err());
    assert_eq!(s.doc.to_json(), before);
    assert!(s.model.state().body("Mesh").unwrap().body.is_mesh());
}

#[test]
fn an_explicit_body_wins_over_the_selection() {
    let mut s = imported_box();
    s.selection = vec![crate::Sel::Body { name: "Missing".into() }];
    run(&mut s, "mesh.convert", json!({"body":"Mesh","tolerance":"0.01 mm"}));
    assert!(!s.model.state().body("Mesh").unwrap().body.is_mesh());
}

#[test]
fn exported_binary_stl_with_a_bore_reconstructs_analytic_faces() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane":"XY","name":"Section"}));
    run(&mut s, "sketch.line", json!({"points":[[0,0],[40,0],[40,20],[20,20],[20,30],[0,30]],"closed":true}));
    run(&mut s, "sketch.circle.center", json!({"center":[10,10],"radius":4}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"sketch":"Section","profiles":[{"point":[30,10]}],"distance":15,"body_name":"Bracket"}));
    let path = std::env::temp_dir().join(format!("solidcraft-mesh-convert-{}.stl", std::process::id()));
    run(&mut s, "file.save_mesh", json!({"path":path}));
    run(&mut s, "doc.open", json!({"path":path}));
    let name = s.model.state().bodies[0].name.clone();
    run(&mut s, "mesh.convert", json!({"body":name,"tolerance":"0.05 mm"}));
    let state = s.model.state();
    let body = &state.bodies[0].body;
    assert!(!body.is_mesh());
    assert!(body.validity().is_empty());
    assert_eq!(solidcraft_kernel::measure(body).unwrap().merged.face_types.get("cylinder"), Some(&1));
    assert_eq!(body.faces(0.01).unwrap().len(), 10);
    std::fs::remove_file(path).unwrap();
}

/// #784: STL is closed as written where faces meet (a fillet, a bored hole): every edge of the
/// file, keyed by its exact f32 ends, is shared by exactly two triangles, as in 3MF.
#[test]
fn stl_export_is_closed_where_faces_meet() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 30, "width": 20, "height": 10, "operation": "new", "body_name": "BoxFillet"}));
    run(&mut s, "solid.fillet", json!({"body": "BoxFillet", "edges": [[15, 0, 10]], "radius": 2}));
    run(&mut s, "solid.cylinder", json!({"diameter": 30, "height": 12, "base": [60, 0, 0], "operation": "new", "body_name": "CylHole"}));
    run(&mut s, "solid.hole", json!({"position": [60, 0, 12], "diameter": 6}));
    let state = s.model.state();
    for name in ["BoxFillet", "CylHole"] {
        let body = &state.bodies.iter().find(|b| b.name == name).unwrap().body;
        let expected = solidcraft_kernel::measure(body).unwrap().volume;
        for format in [solidcraft_io::Format::StlBinary, solidcraft_io::Format::StlAscii] {
            let bytes = solidcraft_io::export(&state, &[name.to_string()], format, name).unwrap();
            let tris = solidcraft_io::read_stl(&bytes).unwrap();
            let key = |p: [f32; 3]| p.map(f32::to_bits);
            let mut edges = std::collections::HashMap::new();
            for t in &tris {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let (a, b) = (key(a), key(b));
                    *edges.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0) += 1;
                }
            }
            let open = edges.values().filter(|n| **n != 2).count();
            assert_eq!(open, 0, "{name} {format:?}: {open} edges not shared by exactly two triangles");
            let p = |v: [f32; 3]| Vec3::new(v[0] as f64, v[1] as f64, v[2] as f64);
            let v: f64 = tris.iter().map(|t| p(t[0]).dot(p(t[1]).cross(p(t[2]))) / 6.0).sum();
            assert!((v - expected).abs() <= expected * 1e-3, "{name} {format:?}: STL volume {v}, body {expected}");
            let mf = solidcraft_io::export(&state, &[name.to_string()], solidcraft_io::Format::ThreeMf, name).unwrap();
            let (objs, _) = solidcraft_io::read_3mf(&mf).unwrap();
            assert_eq!(tris.len(), objs.iter().map(|o| o.triangles.len()).sum::<usize>(), "{name} {format:?}: STL and 3MF triangles");
        }
    }
}

#[test]
fn save_mesh_rejects_unknown_format_and_slicer_without_writing() {
    let mut s = imported_box();
    let path = std::env::temp_dir().join(format!("solidcraft-save-mesh-rejected-{}.stl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    for params in [json!({"path":path,"format":"nope"}), json!({"path":path,"slicer":"no such slicer anywhere"})] {
        assert!(s.execute("file.save_mesh", &params).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn save_mesh_writes_stl_and_3mf() {
    let mut s = imported_box();
    let dir = std::env::temp_dir();
    for (ext, format) in [("stl", None), ("3mf", None), ("dat", Some("3mf"))] {
        let path = dir.join(format!("solidcraft-save-mesh-format-{}.{ext}", std::process::id()));
        let mut params = json!({"path": path});
        if let Some(f) = format {
            params["format"] = json!(f);
        }
        s.execute("file.save_mesh", &params).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // A 3MF file is a zip archive; a binary STL starts with its 80-byte header.
        assert_eq!(bytes.starts_with(b"PK"), ext == "3mf" || format == Some("3mf"), "{ext}");
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn installed_slicers_are_listed() {
    let mut s = imported_box();
    let r = s.execute("file.slicers", &json!({})).unwrap();
    assert!(r["slicers"].is_array());
}

/// Refinement must reach the file writers, including ASCII STL and tagged 3MF. A cylinder's
/// radial chord error is analytic: r minus the radius at the midpoint of a wall edge.
#[test]
fn save_mesh_surface_deviation_refines_exported_curves() {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter":100,"height":20,"operation":"new","body_name":"Round"}));
    run(&mut s, "solid.box", json!({"length":10,"width":10,"height":10,"corner":[200,0,0],"operation":"new","body_name":"Other"}));
    let before = s.doc.to_json();
    for (format, ascii) in [("stl", false), ("stl", true), ("3mf", false), ("3mf-tagged", false)] {
        let path = std::env::temp_dir().join(format!("solidcraft-deviation-{}-{format}-{ascii}", std::process::id()));
        let mut counts = Vec::new();
        for deviation in [0.5, 0.01] {
            run(&mut s, "file.save_mesh", json!({"path":path,"format":format,"ascii":ascii,"bodies":["Round"],"surface_deviation":deviation}));
            let bytes = std::fs::read(&path).unwrap();
            let triangles: Vec<[Vec3; 3]> = if format == "stl" {
                solidcraft_io::read_stl(&bytes).unwrap().into_iter().map(|t| t.map(|p| Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64))).collect()
            } else {
                let (objects, _) = solidcraft_io::read_3mf(&bytes).unwrap();
                assert_eq!(objects.len(), 1, "only the chosen body is exported");
                let o = &objects[0];
                o.triangles.iter().map(|t| t.map(|i| o.positions[i as usize])).collect()
            };
            let mut error = 0.0_f64;
            for t in &triangles {
                assert!(t.iter().all(|p| p.x <= 50.0001), "the unselected box leaked into the file");
                if (t[0].z - t[1].z).abs() < 1e-5 && (t[0].z - t[2].z).abs() < 1e-5 {
                    continue;
                }
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let m = (a + b) * 0.5;
                    error = error.max(50.0 - m.x.hypot(m.y));
                }
            }
            assert!(error > 0.0 && error <= deviation * 1.05 + 1e-4, "{format}/{ascii}, deviation {deviation}: radial error {error}");
            eprintln!("{format}/{ascii}: deviation={deviation}, triangles={}, max radial chord error={error} mm", triangles.len());
            counts.push(triangles.len());
        }
        assert!(counts[1] > counts[0], "{format}/{ascii}: {counts:?}");
        std::fs::remove_file(path).unwrap();
    }
    assert_eq!(s.doc.to_json(), before, "export does not change the design");
}

#[test]
fn save_mesh_default_matches_existing_export_and_bad_deviation_never_overwrites() {
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter":40,"height":10,"operation":"new"}));
    let path = std::env::temp_dir().join(format!("solidcraft-deviation-default-{}.stl", std::process::id()));
    run(&mut s, "file.save_mesh", json!({"path":path}));
    let name = path.file_stem().unwrap().to_str().unwrap();
    let expected =
        solidcraft_io::export_with_metadata(&s.doc.painted(&s.world_state()), &[], solidcraft_io::Format::StlBinary, name, &s.doc.metadata).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    let before = s.doc.to_json();
    for bad in [json!(0), json!(-1), json!(0.00001), json!(1001), json!(1e100), json!("0.01"), json!("NaN"), json!(true), json!(null), json!([])] {
        for command in ["file.save_mesh", "file.export"] {
            assert!(s.execute(command, &json!({"path":path,"surface_deviation":bad})).is_err(), "{command}: {bad}");
            assert_eq!(std::fs::read(&path).unwrap(), expected);
            assert_eq!(s.doc.to_json(), before);
        }
    }
    assert!(s.execute("file.export", &json!({"path":path,"format":"step","surface_deviation":0.01})).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    // The general export command shares the same refinement and writer.
    run(&mut s, "file.export", json!({"path":path,"surface_deviation":0.5}));
    let coarse = solidcraft_io::read_stl(&std::fs::read(&path).unwrap()).unwrap().len();
    run(&mut s, "file.export", json!({"path":path,"surface_deviation":0.01}));
    assert!(solidcraft_io::read_stl(&std::fs::read(&path).unwrap()).unwrap().len() > coarse);
    let mut obj_counts = Vec::new();
    for deviation in [0.5, 0.01] {
        run(&mut s, "file.export", json!({"path":path,"format":"obj","surface_deviation":deviation}));
        obj_counts.push(std::fs::read_to_string(&path).unwrap().lines().filter(|line| line.starts_with("f ")).count());
    }
    assert!(obj_counts[1] > obj_counts[0]);
    std::fs::remove_file(path).unwrap();
}
