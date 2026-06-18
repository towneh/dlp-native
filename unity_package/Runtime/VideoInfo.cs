using System.Collections.Generic;
using Newtonsoft.Json;

namespace YtDlp
{
    public sealed class VideoInfo
    {
        [JsonProperty("id")]                 public string Id { get; set; }
        [JsonProperty("title")]              public string Title { get; set; }
        [JsonProperty("webpage_url")]        public string WebpageUrl { get; set; }
        [JsonProperty("duration")]           public double? Duration { get; set; }
        [JsonProperty("thumbnail")]          public string Thumbnail { get; set; }
        [JsonProperty("uploader")]           public string Uploader { get; set; }
        [JsonProperty("description")]        public string Description { get; set; }
        [JsonProperty("subtitles")]          public Dictionary<string, List<SubtitleTrack>> Subtitles { get; set; }
        [JsonProperty("automatic_captions")] public Dictionary<string, List<SubtitleTrack>> AutomaticCaptions { get; set; }
        [JsonProperty("formats")]            public List<Format> Formats { get; set; }
        [JsonProperty("url")]                public string DirectUrl { get; set; }
    }

    public sealed class Format
    {
        [JsonProperty("format_id")]      public string FormatId { get; set; }
        [JsonProperty("url")]            public string Url { get; set; }
        [JsonProperty("ext")]            public string Ext { get; set; }
        [JsonProperty("vcodec")]         public string VCodec { get; set; }
        [JsonProperty("acodec")]         public string ACodec { get; set; }
        [JsonProperty("width")]          public int? Width { get; set; }
        [JsonProperty("height")]         public int? Height { get; set; }
        [JsonProperty("tbr")]            public double? TotalBitrate { get; set; }
        [JsonProperty("vbr")]            public double? VideoBitrate { get; set; }
        [JsonProperty("abr")]            public double? AudioBitrate { get; set; }
        [JsonProperty("filesize")]       public long? FileSize { get; set; }
        [JsonProperty("filesize_approx")]public long? FileSizeApprox { get; set; }
        [JsonProperty("protocol")]       public string Protocol { get; set; }
    }

    public sealed class SubtitleTrack
    {
        [JsonProperty("url")]  public string Url { get; set; }
        [JsonProperty("ext")]  public string Ext { get; set; }
        [JsonProperty("name")] public string Name { get; set; }
    }

    public sealed class ExtractOptions
    {
        [JsonProperty("format", NullValueHandling = NullValueHandling.Ignore)]
        public string Format { get; set; }

        [JsonProperty("geo_bypass_country", NullValueHandling = NullValueHandling.Ignore)]
        public string GeoBypassCountry { get; set; }
    }
}
