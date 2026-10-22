use acadrust::entities::{Circle, Line, Text};
use acadrust::{CadDocument, DwgWriter, DxfVersion, EntityType, Layer, Vector3};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    for (name, version) in [("2000", DxfVersion::AC1015), ("2018", DxfVersion::AC1032)] {
        let mut doc = CadDocument::with_version(version);
        doc.header.insertion_units = 4;
        doc.header.measurement = 1;
        doc.layers.add(Layer::new("Geometry"))?;
        let mut line = Line::from_points(Vector3::new(10.0, 20.0, 0.0), Vector3::new(110.0, 70.0, 0.0));
        line.common.layer = "Geometry".into();
        doc.add_entity(EntityType::Line(line))?;
        doc.add_entity(EntityType::Circle(Circle::from_center_radius(Vector3::new(40.0, 40.0, 0.0), 10.0)))?;
        doc.add_entity(EntityType::Text(Text::with_value("VectorCraft CAD", Vector3::new(10.0, 80.0, 0.0))))?;
        std::fs::write(format!("drawing-{name}.dwg"), DwgWriter::write_to_vec(&doc)?)?;
    }
    Ok(())
}
