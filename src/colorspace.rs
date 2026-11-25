//! Stride-aware conversion of common USB HDMI pixel formats into I420 for H.264.
//! Pure Rust scalar baseline for validation. NEON optimization is optional and
//! must be benchmarked against this reference implementation on a real Pi 5.
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I420Frame {
    pub width: usize,
    pub height: usize,
    pub timestamp_us: u64,
    /// Contiguous Y, U, V planes, in that order; each plane tightly packed.
    pub bytes: Vec<u8>,
}
impl I420Frame {
    pub fn y(&self) -> &[u8] { &self.bytes[..self.width * self.height] }
    pub fn u(&self) -> &[u8] {
        let n=self.width * self.height;
        &self.bytes[n..n+n/4]
    }
    pub fn v(&self) -> &[u8] {
        let n=self.width * self.height;
        &self.bytes[n+n/4..]
    }
}

fn validate(width: usize, height: usize, stride: usize, minimum: usize, length: usize) -> io::Result<()> {
    if width < 2 || height < 2 || width > 3840 || height > 2160 ||
        width % 2 != 0 || height % 2 != 0 || stride < minimum ||
        stride.checked_mul(height).is_none_or(|need| need > length) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid image geometry, stride or buffer length"));
    }
    Ok(())
}

/// Convert packed 4:2:2 YUYV bytes to planar 4:2:0 I420.
/// The two vertical U/V samples are averaged to create chroma at half height.
pub fn yuyv422_to_i420(
    input: &[u8], width: usize, height: usize, stride: usize, timestamp_us: u64,
) -> io::Result<I420Frame> {
    let row_bytes=width.checked_mul(2).ok_or_else(|| io::Error::other("dimension overflow"))?;
    validate(width,height,stride,row_bytes,input.len())?;
    let y_len=width * height;
    let chroma_len=y_len/4;
    let mut data=vec![0u8; y_len + 2*chroma_len];
    let (y,chromas)=data.split_at_mut(y_len);
    let (u,v)=chromas.split_at_mut(chroma_len);

    for pair_y in 0..height/2 {
        let row0=pair_y*2*stride;
        let row1=row0+stride;
        for pair_x in 0..width/2 {
            let col=pair_x*4;
            let i0=row0+col;
            let i1=row1+col;
            let output_x=pair_x*2;
            let y0=pair_y*2*width+output_x;
            let y1=y0+width;
            y[y0]=input[i0];
            y[y0+1]=input[i0+2];
            y[y1]=input[i1];
            y[y1+1]=input[i1+2];
            let chroma_index=pair_y*(width/2)+pair_x;
            u[chroma_index]=((u16::from(input[i0+1])+u16::from(input[i1+1])+1)/2) as u8;
            v[chroma_index]=((u16::from(input[i0+3])+u16::from(input[i1+3])+1)/2) as u8;
        }
    }
    Ok(I420Frame {width,height,timestamp_us,bytes:data})
}

/// Convert NV12 (full-size luma and interleaved UV 4:2:0) to planar I420.
/// Both Y and UV rows use the provided stride; an alternate UV stride is
/// supported for devices with different luma/chroma pitches.
pub fn nv12_to_i420(
    input: &[u8], width: usize, height: usize,
    y_stride: usize, uv_stride: usize, timestamp_us: u64,
) -> io::Result<I420Frame> {
    let y_bytes=y_stride.checked_mul(height).ok_or_else(||invalid())?;
    let uv_bytes=uv_stride.checked_mul(height/2).ok_or_else(||invalid())?;
    let required=y_bytes.checked_add(uv_bytes).ok_or_else(||invalid())?;
    if width<2 || height<2 || width>3840 || height>2160 || width%2!=0 || height%2!=0 ||
        y_stride<width || uv_stride<width || input.len()<required {
        return Err(invalid());
    }
    let y_len=width*height;
    let mut data=vec![0u8; y_len+y_len/2];
    let (y,uv)=data.split_at_mut(y_len);
    let (u,v)=uv.split_at_mut(y_len/4);
    for row in 0..height {
        y[row*width..(row+1)*width].copy_from_slice(&input[row*y_stride..row*y_stride+width]);
    }
    for row in 0..height/2 {
        for col in 0..width/2 {
            let off=y_bytes+row*uv_stride+col*2;
            let pos=row*(width/2)+col;
            u[pos]=input[off];
            v[pos]=input[off+1];
        }
    }
    Ok(I420Frame {width,height,timestamp_us,bytes:data})
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, "invalid NV12 dimensions or plane strides") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yuyv_chroma_is_vertically_subsampled() {
        // Y0=10 U0=30 Y1=20 V0=50; Y2=40 U1=70 Y3=60 V1=90.
        let input=[10,30,20,50,40,70,60,90];
        let out=yuyv422_to_i420(&input,2,2,4,123).unwrap();
        assert_eq!(out.timestamp_us,123);
        assert_eq!(out.y(),&[10,20,40,60]);
        assert_eq!(out.u(),&[50]);
        assert_eq!(out.v(),&[70]);
    }
    #[test]
    fn yuyv_respects_padding_and_does_not_mix_rows() {
        let input=[1,10,2,20,0xee,0xee,3,30,4,40,0xff,0xff];
        let out=yuyv422_to_i420(&input,2,2,6,0).unwrap();
        assert_eq!(out.y(),&[1,2,3,4]);
        assert_eq!(out.u(),&[20]);
        assert_eq!(out.v(),&[30]);
    }
    #[test]
    fn nv12_splits_uv_interleaving_with_stride() {
        let input=[1,2,99,99, 3,4,88,88, 20,30,66,66];
        let out=nv12_to_i420(&input,2,2,4,4,33).unwrap();
        assert_eq!(out.y(),&[1,2,3,4]);
        assert_eq!(out.u(),&[20]);
        assert_eq!(out.v(),&[30]);
        assert_eq!(out.timestamp_us,33);
    }
    #[test]
    fn rejects_truncated_or_odd_sized_frames() {
        assert!(yuyv422_to_i420(&[],2,2,4,0).is_err());
        assert!(yuyv422_to_i420(&[0;32],3,2,8,0).is_err());
        assert!(nv12_to_i420(&[0;8],2,3,2,2,0).is_err());
        assert!(nv12_to_i420(&[0;4],2,2,2,2,0).is_err());
    }
}
