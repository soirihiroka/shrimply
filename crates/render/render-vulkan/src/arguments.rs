use super::KernelLayout;

pub struct Arguments {
    pub(super) layout: &'static KernelLayout,
    pub(super) code: &'static [u8],
    pub(super) specialization: u32,
    pub(super) bytes: Vec<u8>,
    pub(super) portable: bool,
}
impl Arguments {
    pub fn set(&mut self, name: &str, bytes: &[u8]) -> Result<&mut Self, String> {
        let field = self
            .layout
            .fields
            .iter()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("Unknown {} argument {name}", self.layout.name))?;
        if field.boolean && field.size == size_of::<u32>() && bytes.len() == 1 {
            self.bytes[field.offset..field.offset + field.size]
                .copy_from_slice(&u32::from(bytes[0]).to_ne_bytes());
            return Ok(self);
        }
        if self.portable
            && field.unsigned
            && field.size == size_of::<u32>()
            && bytes.len() == size_of::<u64>()
        {
            let value = u64::from_ne_bytes(bytes.try_into().expect("64-bit argument"));
            let value = u32::try_from(value).map_err(|_| {
                format!(
                    "{} argument {name} exceeds the GPU's 32-bit index range",
                    self.layout.name
                )
            })?;
            self.bytes[field.offset..field.offset + field.size]
                .copy_from_slice(&value.to_ne_bytes());
            return Ok(self);
        }
        if field.size != bytes.len() {
            return Err(format!(
                "{} argument {name} requires {} bytes, received {}",
                self.layout.name,
                field.size,
                bytes.len()
            ));
        }
        self.bytes[field.offset..field.offset + field.size].copy_from_slice(bytes);
        Ok(self)
    }
    pub fn set_matrix3(
        &mut self,
        name: &str,
        matrix: shrimply_render_core::math::Mat3,
    ) -> Result<&mut Self, String> {
        for (axis, column) in [
            ("x_axis", matrix.x_axis),
            ("y_axis", matrix.y_axis),
            ("z_axis", matrix.z_axis),
        ] {
            let field_name = format!("{name}.{axis}");
            let length = self
                .layout
                .fields
                .iter()
                .find(|f| f.name == field_name)
                .ok_or_else(|| format!("Missing Vulkan matrix argument {field_name}"))?
                .size;
            let bytes = column.extend(0.0).to_array().map(f32::to_ne_bytes);
            self.set(&field_name, &bytes.as_flattened()[..length])?;
        }
        Ok(self)
    }
}
