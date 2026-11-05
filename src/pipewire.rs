//! What the session audio graph actually says about a stream.
//!
//! A shared graph can pass samples through untouched, and it cannot be told
//! apart from one that converts them by listening alone: "nothing else is
//! playing" is necessary, not sufficient. PipeWire publishes the answer —
//! the nodes, the links and the negotiated link format — so this module reads
//! that instead of guessing.
//!
//! It reports facts only. Whether a verified pass-through may be called
//! bit-perfect is a separate decision, and nothing here changes a verdict.

use serde_json::Value;

/// One node on the way from the client's stream to the device.
#[derive(Clone, Debug, PartialEq)]
pub struct Hop {
    pub name: String,
    pub class: String,
    /// Format the link into this node negotiated, when PipeWire reported one.
    pub link_format: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    /// The client's stream node, as PipeWire names it.
    pub stream: String,
    /// Rate and sample format PipeWire says the stream negotiated.
    pub stream_format: Option<String>,
    /// The chain from the stream to the sink, in order.
    pub hops: Vec<Hop>,
    /// Nodes that convert or resample, which make a pass-through impossible.
    pub converters: Vec<String>,
    /// Other clients feeding the same sink at the same time.
    pub other_clients: usize,
}

impl Graph {
    /// A pass-through is possible only when every link along the chain carries
    /// the stream's own samples: no converter node, and a link format that is
    /// not the graph's float.
    pub fn pass_through_possible(&self) -> bool {
        self.converters.is_empty()
            && self.other_clients == 0
            && self
                .hops
                .iter()
                .all(|hop| hop.link_format.as_deref().is_some_and(|f| f != "F32P"))
    }
}

fn class_of(node: &Value) -> String {
    node["info"]["props"]["media.class"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn name_of(node: &Value) -> String {
    let props = &node["info"]["props"];
    props["node.name"]
        .as_str()
        .or_else(|| props["application.name"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("node{}", node["id"]))
}

/// The audio format a node negotiated, as `48000:S32LE:2`.
fn format_of(node: &Value) -> Option<String> {
    let params = node["info"]["params"]["Format"].as_array()?;
    let entry = params
        .iter()
        .find(|e| e["mediaType"].as_str() == Some("audio"))?;
    let rate = entry["rate"].as_i64()?;
    let format = entry["format"].as_str()?;
    let channels = entry["channels"].as_i64().unwrap_or(0);
    Some(format!("{rate}:{format}:{channels}"))
}

fn link_format_of(link: &Value) -> Option<String> {
    link["info"]["format"]["format"]
        .as_str()
        .map(str::to_string)
}

/// Read a `pw-dump` document and follow the first stream whose name contains
/// `needle` to the sink it feeds. `None` when the graph does not show it.
pub fn inspect(dump: &str, needle: &str) -> Option<Graph> {
    let nodes: Vec<Value> = serde_json::from_str::<Vec<Value>>(dump).ok()?;
    let by_id: std::collections::HashMap<i64, &Value> = nodes
        .iter()
        .filter(|n| n["type"].as_str() == Some("PipeWire:Interface:Node"))
        .filter_map(|n| n["id"].as_i64().map(|id| (id, n)))
        .collect();
    let links: Vec<&Value> = nodes
        .iter()
        .filter(|n| n["type"].as_str() == Some("PipeWire:Interface:Link"))
        .collect();

    let (stream_id, stream_node) = by_id
        .iter()
        .find(|(_, node)| name_of(node).contains(needle))
        .map(|(id, node)| (*id, *node))?;

    let mut hops = Vec::new();
    let mut converters = Vec::new();
    let mut current = stream_id;
    let mut sink = None;
    for _ in 0..16 {
        let Some(link) = links
            .iter()
            .find(|l| l["info"]["output-node-id"].as_i64() == Some(current))
        else {
            break;
        };
        let next = link["info"]["input-node-id"].as_i64()?;
        let node = by_id.get(&next)?;
        let class = class_of(node);
        hops.push(Hop {
            name: name_of(node),
            class: class.clone(),
            link_format: link_format_of(link),
        });
        let name = name_of(node);
        if name.contains("audioconvert") || name.contains("audioresample") {
            converters.push(name);
        }
        current = next;
        if class == "Audio/Sink" {
            sink = Some(next);
            break;
        }
    }
    let sink = sink?;

    // A stereo stream reaches the sink with one link per port, so count the
    // client nodes rather than the links: two links from one stream are not
    // two clients.
    let mut peers: Vec<i64> = links
        .iter()
        .filter(|l| l["info"]["input-node-id"].as_i64() == Some(sink))
        .filter_map(|l| l["info"]["output-node-id"].as_i64())
        .filter(|id| {
            by_id
                .get(id)
                .is_some_and(|n| class_of(n) == "Stream/Output/Audio")
        })
        .collect();
    peers.sort_unstable();
    peers.dedup();
    let other_clients = peers.iter().filter(|id| **id != stream_id).count();

    Some(Graph {
        stream: name_of(stream_node),
        stream_format: format_of(stream_node),
        hops,
        converters,
        other_clients,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, name: &str, class: &str, format: Option<(&str, i64)>) -> Value {
        let mut n = serde_json::json!({
            "id": id,
            "type": "PipeWire:Interface:Node",
            "info": {"props": {"node.name": name, "media.class": class}}
        });
        if let Some((fmt, rate)) = format {
            n["info"]["params"]["Format"] = serde_json::json!([{"mediaType": "audio", "rate": rate, "format": fmt, "channels": 2}]);
        }
        n
    }

    fn link(from: i64, to: i64, format: &str) -> Value {
        serde_json::json!({
            "type": "PipeWire:Interface:Link",
            "info": {"output-node-id": from, "input-node-id": to, "format": {"format": format}}
        })
    }

    fn dump(extra: Vec<Value>) -> String {
        let mut v = vec![
            node(
                10,
                "alsa_playback.melbad",
                "Stream/Output/Audio",
                Some(("S32LE", 48000)),
            ),
            node(20, "alsa_output.hdmi", "Audio/Sink", None),
            link(10, 20, "F32P"),
        ];
        v.extend(extra);
        serde_json::to_string(&v).unwrap()
    }

    #[test]
    fn a_direct_link_that_is_already_float_is_not_a_pass_through() {
        let g = inspect(&dump(vec![]), "melbad").expect("the stream is there");
        assert_eq!(g.stream_format.as_deref(), Some("48000:S32LE:2"));
        assert_eq!(g.hops.len(), 1);
        assert_eq!(g.hops[0].class, "Audio/Sink");
        assert!(
            !g.pass_through_possible(),
            "the link carries the graph's float"
        );
    }

    #[test]
    fn a_converter_on_the_way_rules_a_pass_through_out() {
        // A chain with the converter in it, so the first link from the stream
        // is not the direct one.
        let v = vec![
            node(
                10,
                "alsa_playback.melbad",
                "Stream/Output/Audio",
                Some(("S32LE", 48000)),
            ),
            node(15, "audioconvert", "Stream/Internal", None),
            node(20, "alsa_output.hdmi", "Audio/Sink", None),
            link(10, 15, "F32P"),
            link(15, 20, "F32P"),
        ];
        let g =
            inspect(&serde_json::to_string(&v).unwrap(), "melbad").expect("the stream is there");
        assert_eq!(g.converters, ["audioconvert"]);
        assert_eq!(
            g.hops.len(),
            2,
            "the chain is followed, not stopped at the first hop"
        );
        assert!(!g.pass_through_possible());
    }

    #[test]
    fn another_client_on_the_same_sink_is_counted() {
        let extra = vec![
            node(30, "firefox", "Stream/Output/Audio", None),
            link(30, 20, "F32P"),
        ];
        let g = inspect(&dump(extra), "melbad").expect("the stream is there");
        assert_eq!(g.other_clients, 1);
        assert!(!g.pass_through_possible());
    }

    /// One stream reaches the sink with one link per port. Counting links made
    /// every stereo stream look as if a second client shared the device.
    #[test]
    fn one_stereo_streams_two_port_links_are_not_a_second_client() {
        let mut v = vec![
            node(
                10,
                "alsa_playback.melbad",
                "Stream/Output/Audio",
                Some(("S32LE", 48000)),
            ),
            node(20, "alsa_output.hdmi", "Audio/Sink", None),
            link(10, 20, "S32LE"),
            link(10, 20, "S32LE"),
        ];
        let g = inspect(&serde_json::to_string(&v).unwrap(), "melbad").unwrap();
        assert_eq!(g.other_clients, 0);
        assert!(g.pass_through_possible());
        v.clear();
    }

    #[test]
    fn an_integer_link_with_no_converter_and_no_peer_could_pass_through() {
        let mut v = vec![
            node(
                10,
                "alsa_playback.melbad",
                "Stream/Output/Audio",
                Some(("S32LE", 48000)),
            ),
            node(20, "alsa_output.hdmi", "Audio/Sink", None),
            link(10, 20, "S32LE"),
        ];
        let text = serde_json::to_string(&v).unwrap();
        let g = inspect(&text, "melbad").expect("the stream is there");
        assert!(g.pass_through_possible(), "{g:?}");
        v.clear();
    }

    #[test]
    fn a_graph_without_the_stream_says_nothing() {
        assert!(inspect(&dump(vec![]), "nonexistent").is_none());
        assert!(inspect("not json", "melbad").is_none());
    }
}
