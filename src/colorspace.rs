//! Stride-aware conversion of common USB HDMI pixel formats into I420 for H.264.
//! AArch64 NEON conversion with a portable scalar reference and tail path.
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
        !width.is_multiple_of(2) || !height.is_multiple_of(2) || stride < minimum ||
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
    let mut frame = I420Frame { width, height, timestamp_us, bytes: Vec::new() };
    yuyv422_to_i420_into(input, width, height, stride, timestamp_us, &mut frame)?;
    Ok(frame)
}

/// Convert into reusable storage without changing the destination on invalid input.
pub fn yuyv422_to_i420_into(
    input: &[u8], width: usize, height: usize, stride: usize, timestamp_us: u64,
    frame: &mut I420Frame,
) -> io::Result<()> {
    let row_bytes=width.checked_mul(2).ok_or_else(|| io::Error::other("dimension overflow"))?;
    validate(width,height,stride,row_bytes,input.len())?;
    let y_len=width * height;
    let chroma_len=y_len/4;
    frame.bytes.resize(y_len + 2*chroma_len, 0);
    let (y,chromas)=frame.bytes.split_at_mut(y_len);
    let (u,v)=chromas.split_at_mut(chroma_len);

    for pair_y in 0..height/2 {
        let row0=pair_y*2*stride;
        let row1=row0+stride;
        let y0=pair_y*2*width;
        let chroma=pair_y*(width/2);
        let (top,bottom)=y[y0..y0+2*width].split_at_mut(width);
        convert_yuyv_rows(
            &input[row0..row0+row_bytes], &input[row1..row1+row_bytes],
            top, bottom, &mut u[chroma..chroma+width/2],
            &mut v[chroma..chroma+width/2],
        );
    }
    frame.width = width;
    frame.height = height;
    frame.timestamp_us = timestamp_us;
    Ok(())
}

/// Convert NV12 (full-size luma and interleaved UV 4:2:0) to planar I420.
/// Both Y and UV rows use the provided stride; an alternate UV stride is
/// supported for devices with different luma/chroma pitches.
pub fn nv12_to_i420(
    input: &[u8], width: usize, height: usize,
    y_stride: usize, uv_stride: usize, timestamp_us: u64,
) -> io::Result<I420Frame> {
    let mut frame = I420Frame { width, height, timestamp_us, bytes: Vec::new() };
    nv12_to_i420_into(input, width, height, (y_stride, uv_stride), timestamp_us, &mut frame)?;
    Ok(frame)
}

/// Convert NV12 into reusable storage with independent luma and chroma strides.
pub fn nv12_to_i420_into(
    input: &[u8], width: usize, height: usize,
    strides: (usize, usize), timestamp_us: u64, frame: &mut I420Frame,
) -> io::Result<()> {
    let (y_stride, uv_stride) = strides;
    let y_bytes=y_stride.checked_mul(height).ok_or_else(invalid)?;
    let uv_bytes=uv_stride.checked_mul(height/2).ok_or_else(invalid)?;
    let required=y_bytes.checked_add(uv_bytes).ok_or_else(invalid)?;
    if width<2 || height<2 || width>3840 || height>2160 || !width.is_multiple_of(2) || !height.is_multiple_of(2) ||
        y_stride<width || uv_stride<width || input.len()<required {
        return Err(invalid());
    }
    let y_len=width*height;
    frame.bytes.resize(y_len+y_len/2, 0);
    let (y,uv)=frame.bytes.split_at_mut(y_len);
    let (u,v)=uv.split_at_mut(y_len/4);
    for row in 0..height {
        y[row*width..(row+1)*width].copy_from_slice(&input[row*y_stride..row*y_stride+width]);
    }
    for row in 0..height/2 {
        let off=y_bytes+row*uv_stride;
        let pos=row*(width/2);
        split_uv_row(&input[off..off+width],
            &mut u[pos..pos+width/2], &mut v[pos..pos+width/2]);
    }
    frame.width = width;
    frame.height = height;
    frame.timestamp_us = timestamp_us;
    Ok(())
}

// AArch64 NEON handles 16 pixel pairs per iteration. The scalar tail is
// also the portable implementation; no SIMD crate or build flag is required.
fn convert_yuyv_rows(a: &[u8], b: &[u8], y0: &mut [u8], y1: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    let pairs = {
        use std::arch::aarch64::*;
        let blocks = u.len() / 16;
        for i in 0..blocks {
            let p = i * 16;
            // SAFETY: validated row slices hold four input bytes and two Y
            // output bytes per pair, plus one byte in each chroma plane.
            // Each block is wholly inside the slices; NEON loads are unaligned.
            unsafe {
                let x = vld4q_u8(a.as_ptr().add(p * 4));
                let z = vld4q_u8(b.as_ptr().add(p * 4));
                vst2q_u8(y0.as_mut_ptr().add(p * 2), uint8x16x2_t(x.0, x.2));
                vst2q_u8(y1.as_mut_ptr().add(p * 2), uint8x16x2_t(z.0, z.2));
                vst1q_u8(u.as_mut_ptr().add(p), vrhaddq_u8(x.1, z.1));
                vst1q_u8(v.as_mut_ptr().add(p), vrhaddq_u8(x.3, z.3));
            }
        }
        blocks * 16
    };
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    let pairs = 0;
    scalar_yuyv_rows(&a[pairs*4..], &b[pairs*4..], &mut y0[pairs*2..],
        &mut y1[pairs*2..], &mut u[pairs..], &mut v[pairs..]);
}

fn scalar_yuyv_rows(a: &[u8], b: &[u8], y0: &mut [u8], y1: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    for i in 0..u.len() {
        let p=i*4;
        y0[i*2]=a[p];
        y0[i*2+1]=a[p+2];
        y1[i*2]=b[p];
        y1[i*2+1]=b[p+2];
        u[i]=(u16::from(a[p+1])+u16::from(b[p+1])).div_ceil(2) as u8;
        v[i]=(u16::from(a[p+3])+u16::from(b[p+3])).div_ceil(2) as u8;
    }
}

fn split_uv_row(input: &[u8], u: &mut [u8], v: &mut [u8]) {
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    let pairs = {
        use std::arch::aarch64::*;
        let blocks = u.len() / 16;
        for i in 0..blocks {
            let p = i * 16;
            // SAFETY: each complete block reads 32 bytes and writes 16
            // bytes to each disjoint output plane, within the row slices.
            unsafe {
                let uv = vld2q_u8(input.as_ptr().add(p * 2));
                vst1q_u8(u.as_mut_ptr().add(p), uv.0);
                vst1q_u8(v.as_mut_ptr().add(p), uv.1);
            }
        }
        blocks * 16
    };
    #[cfg(not(all(target_arch = "aarch64", target_feature = "neon")))]
    let pairs = 0;
    scalar_split_uv_row(&input[pairs*2..], &mut u[pairs..], &mut v[pairs..]);
}

fn scalar_split_uv_row(input: &[u8], u: &mut [u8], v: &mut [u8]) {
    for i in 0..u.len() {
        u[i]=input[i*2];
        v[i]=input[i*2+1];
    }
}

fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidInput, "invalid NV12 dimensions or plane strides") }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_rows_match_scalar_with_tails_and_unaligned_input() {
        for width in [2usize, 30, 32, 34, 62, 64, 66, 1920] {
            let a: Vec<u8> = (0..width*2+1).map(|i| (i*37+19) as u8).collect();
            let b: Vec<u8> = (0..width*2+1).map(|i| (i*59+20) as u8).collect();
            let (a,b)=(&a[1..], &b[1..]);
            let mut actual = (vec![0; width],vec![0; width],vec![0; width/2],vec![0; width/2]);
            let mut expected = actual.clone();
            convert_yuyv_rows(a,b,&mut actual.0,&mut actual.1,&mut actual.2,&mut actual.3);
            scalar_yuyv_rows(a,b,&mut expected.0,&mut expected.1,&mut expected.2,&mut expected.3);
            assert_eq!(actual,expected,"YUYV width {width}");
            split_uv_row(&a[..width],&mut actual.2,&mut actual.3);
            scalar_split_uv_row(&a[..width],&mut expected.2,&mut expected.3);
            assert_eq!(actual,expected,"NV12 width {width}");
        }
    }
    #[test]
    fn padded_vector_frames_match_scalar() {
        let width=66;
        let height=4;
        let stride=width*2+7;
        let input: Vec<u8> = (0..stride*height).map(|i| (i*43+11) as u8).collect();
        let actual=yuyv422_to_i420(&input,width,height,stride,23).unwrap();
        let mut expected=vec![0; width*height*3/2];
        let (y,chroma)=expected.split_at_mut(width*height);
        let (u,v)=chroma.split_at_mut(width*height/4);
        for row in 0..height/2 {
            let off=row*2*stride;
            let (top,bottom)=y[row*2*width..(row+1)*2*width].split_at_mut(width);
            scalar_yuyv_rows(&input[off..off+width*2], &input[off+stride..off+stride+width*2],
                top,bottom,&mut u[row*width/2..(row+1)*width/2],&mut v[row*width/2..(row+1)*width/2]);
        }
        assert_eq!(actual.bytes,expected);
        assert_eq!(actual.timestamp_us,23);
    }
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
    fn reusable_storage_preserves_conversion_and_rejects_invalid_input() {
        let yuyv = [1, 10, 2, 20, 3, 30, 4, 40];
        let nv12 = [5, 6, 7, 8, 90, 100];
        let mut frame = yuyv422_to_i420(&yuyv, 2, 2, 4, 1).unwrap();
        let ptr = frame.bytes.as_ptr();
        nv12_to_i420_into(&nv12, 2, 2, (2, 2), 2, &mut frame).unwrap();
        assert_eq!(frame.bytes, [5, 6, 7, 8, 90, 100]);
        assert_eq!(frame.timestamp_us, 2);
        assert_eq!(frame.bytes.as_ptr(), ptr);
        yuyv422_to_i420_into(&yuyv, 2, 2, 4, 3, &mut frame).unwrap();
        assert_eq!(frame.bytes, [1, 2, 3, 4, 20, 30]);
        assert_eq!(frame.timestamp_us, 3);
        assert_eq!(frame.bytes.as_ptr(), ptr);
        let expected = frame.clone();
        assert!(yuyv422_to_i420_into(&[], 2, 2, 4, 4, &mut frame).is_err());
        assert_eq!(frame, expected);
    }
    #[test]
    fn rejects_truncated_or_odd_sized_frames() {
        assert!(yuyv422_to_i420(&[],2,2,4,0).is_err());
        assert!(yuyv422_to_i420(&[0;32],3,2,8,0).is_err());
        assert!(nv12_to_i420(&[0;8],2,3,2,2,0).is_err());
        assert!(nv12_to_i420(&[0;4],2,2,2,2,0).is_err());
    }
}
