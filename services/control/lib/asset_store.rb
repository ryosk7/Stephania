# frozen_string_literal: true

require "openssl"
require "fileutils"

# Content-addressed PNG store shared with the engine (crate::assets).
# The engine re-checks the hash and decodes; here we only accept files that
# are structurally PNG with sane dimensions, so garbage never reaches it.
module AssetStore
  MAX_BYTES = 8 * 1024 * 1024
  MAX_DIMENSION = 4096
  MAX_ASSETS = 256
  SIGNATURE = "\x89PNG\r\n\x1a\n".b
  ID = /\A[0-9a-f]{16}\z/

  module_function

  # [width, height] or an error string.
  def inspect_png(data)
    return "not a PNG file" unless data.byteslice(0, 8) == SIGNATURE && data.bytesize >= 33
    return "PNG must start with IHDR" unless data.byteslice(12, 4) == "IHDR"

    width, height = data.byteslice(16, 8).unpack("NN")
    return "PNG dimensions must be 1..#{MAX_DIMENSION}" unless width.between?(1, MAX_DIMENSION) && height.between?(1, MAX_DIMENSION)
    return "PNG has no IEND chunk" unless data.byteslice(-12, 12)&.byteslice(4, 4) == "IEND"

    [width, height]
  end

  def id_for(data) = OpenSSL::Digest::SHA256.hexdigest(data)[0, 16]

  # Writes atomically; group-readable so the engine (t3box group) can read
  # it despite the service's UMask=0077.
  def write(dir, id, data)
    FileUtils.mkdir_p(dir)
    path = File.join(dir, "#{id}.png")
    return path if File.file?(path) && File.size(path) == data.bytesize

    tmp = "#{path}.#{Process.pid}.#{Thread.current.object_id}.tmp"
    File.open(tmp, File::WRONLY | File::CREAT | File::EXCL, 0o640) { |f| f.write(data); f.fsync }
    File.chmod(0o640, tmp)
    File.rename(tmp, path)
    path
  ensure
    File.unlink(tmp) if tmp && File.exist?(tmp)
  end

  def path(dir, id) = File.join(dir, "#{id}.png")

  # Asset ids referenced by image layers of a scene document.
  def referenced(doc)
    return [] unless doc.is_a?(Hash) && doc["layers"].is_a?(Array)

    doc["layers"].filter_map { |l| l["asset"] if l.is_a?(Hash) && l["type"] == "image" && l["asset"].is_a?(String) }.uniq
  end
end
