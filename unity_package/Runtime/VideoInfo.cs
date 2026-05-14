using System.Collections.Generic;
using System.Text.Json.Serialization;

namespace YtDlp
{
    /// <summary>
    /// Subset of yt-dlp's info_dict returned by <see cref="YtDlp.Extract"/>.
    /// Fields are nullable — not all extractors populate every field.
    /// </summary>
    public sealed class VideoInfo
    {
        [JsonPropertyName("id")]
        public string Id { get; set; }

        [JsonPropertyName("title")]
        public string Title { get; set; }

        [JsonPropertyName("webpage_url")]
        public string WebpageUrl { get; set; }

        [JsonPropertyName("duration")]
        public double? Duration { get; set; }

        [JsonPropertyName("thumbnail")]
        public string Thumbnail { get; set; }

        [JsonPropertyName("uploader")]
        public string Uploader { get; set; }

        [JsonPropertyName("description")]
        public string Description { get; set; }

        [JsonPropertyName("formats")]
        public List<Format> Formats { get; set; }

        [JsonPropertyName("url")]
        public string DirectUrl { get; set; }
    }

    /// <summary>Single media format entry from yt-dlp's formats list.</summary>
    public sealed class Format
    {
        [JsonPropertyName("format_id")]
        public string FormatId { get; set; }

        [JsonPropertyName("url")]
        public string Url { get; set; }

        [JsonPropertyName("ext")]
        public string Ext { get; set; }

        [JsonPropertyName("vcodec")]
        public string VCodec { get; set; }

        [JsonPropertyName("acodec")]
        public string ACodec { get; set; }

        [JsonPropertyName("width")]
        public int? Width { get; set; }

        [JsonPropertyName("height")]
        public int? Height { get; set; }

        [JsonPropertyName("tbr")]
        public double? TotalBitrate { get; set; }

        [JsonPropertyName("vbr")]
        public double? VideoBitrate { get; set; }

        [JsonPropertyName("abr")]
        public double? AudioBitrate { get; set; }

        [JsonPropertyName("filesize")]
        public long? FileSize { get; set; }

        [JsonPropertyName("filesize_approx")]
        public long? FileSizeApprox { get; set; }

        [JsonPropertyName("protocol")]
        public string Protocol { get; set; }
    }

    /// <summary>Options passed to <see cref="YtDlp.Extract"/>.</summary>
    public sealed class ExtractOptions
    {
        /// <summary>
        /// Limit the number of formats returned.
        /// Null = return all formats (yt-dlp default).
        /// </summary>
        [JsonPropertyName("format")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string Format { get; set; }

        /// <summary>
        /// Two-letter ISO country code to spoof for geo-restricted content.
        /// </summary>
        [JsonPropertyName("geo_bypass_country")]
        [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
        public string GeoBypassCountry { get; set; }
    }
}
