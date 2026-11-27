# frozen_string_literal: true

# Validation and persistence of the program audio inputs (docs/AUDIO.md).
# Mirrors the engine's rules (crate::audio) so a configuration can be saved
# while the engine is down and applied when it comes back.
module AudioSettings
  IDS = %w[hdmi mic].freeze
  FIELDS = %w[id enabled device channels gain_db muted].freeze
  DEVICE = /\A[A-Za-z0-9,:=._-]{1,96}\z/
  GAIN = (-60.0..12.0)

  module_function

  # Returns [normalised inputs, nil] or [nil, error message].
  def validate(input, stored)
    return [nil, "body must be {inputs: [...]}"] unless input.is_a?(Hash) && input.keys == ["inputs"] && input["inputs"].is_a?(Array)

    items = input["inputs"]
    return [nil, "one or two inputs"] unless (1..2).cover?(items.size)

    merged = stored.transform_values(&:dup)
    seen = []
    items.each do |item|
      return [nil, "each input must be an object"] unless item.is_a?(Hash)
      unknown = item.keys - FIELDS
      return [nil, "unknown field #{unknown.first}"] unless unknown.empty?
      id = item["id"]
      return [nil, "id must be hdmi or mic"] unless IDS.include?(id)
      return [nil, "duplicate input #{id}"] if seen.include?(id)
      seen << id
      cur = merged[id] || {"id" => id, "enabled" => id == "hdmi", "device" => "", "channels" => (id == "hdmi" ? 2 : 1), "gain_db" => 0.0, "muted" => false}
      if item.key?("enabled")
        return [nil, "enabled must be a boolean"] unless [true, false].include?(item["enabled"])
        cur["enabled"] = item["enabled"]
      end
      if item.key?("device")
        d = item["device"]
        return [nil, "device must be an ALSA name like plughw:CARD=Mic,DEV=0"] unless d.is_a?(String) && (d.empty? || d.match?(DEVICE))
        cur["device"] = d
      end
      if item.key?("channels")
        return [nil, "channels must be 1 or 2"] unless [1, 2].include?(item["channels"])
        cur["channels"] = item["channels"]
      end
      if item.key?("gain_db")
        g = item["gain_db"]
        return [nil, "gain_db must be within -60..12"] unless g.is_a?(Numeric) && g.to_f.finite? && GAIN.cover?(g.to_f)
        cur["gain_db"] = g.to_f
      end
      if item.key?("muted")
        return [nil, "muted must be a boolean"] unless [true, false].include?(item["muted"])
        cur["muted"] = item["muted"]
      end
      return [nil, "#{id}: an enabled input needs a device"] if cur["enabled"] && cur["device"].empty?
      merged[id] = cur
    end
    h, m = merged["hdmi"], merged["mic"]
    if h && m && h["enabled"] && m["enabled"] && h["device"] == m["device"]
      return [nil, "hdmi and mic must use different devices"]
    end
    [merged, nil]
  end

  def load(db)
    db[:audio_inputs].all.to_h do |r|
      [r[:id], {"id" => r[:id], "enabled" => r[:enabled], "device" => r[:device], "channels" => r[:channels],
                "gain_db" => r[:gain_db], "muted" => r[:muted]}]
    end
  end

  def save(db, inputs)
    now = Time.now.to_i
    db.transaction do
      inputs.each_value do |i|
        row = {enabled: i["enabled"], device: i["device"], channels: i["channels"], gain_db: i["gain_db"], muted: i["muted"], updated_at_unix: now}
        db[:audio_inputs].where(id: i["id"]).update(row).zero? && db[:audio_inputs].insert(row.merge(id: i["id"]))
      end
    end
  end

  # IPC params for audio.configure from stored rows (nil when none stored).
  def params(inputs)
    inputs.empty? ? nil : {inputs: inputs.values}
  end
end
