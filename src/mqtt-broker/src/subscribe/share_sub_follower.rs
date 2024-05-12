use super::{manager::SubscribeManager, share_sub_leader::publish_to_client, subscribe::{is_share_sub_rewrite_publish, share_sub_rewrite_publish_flag}};
use crate::{
    core::metadata_cache::MetadataCacheManager,
    server::{tcp::packet::ResponsePackage, MQTTProtocol},
    subscribe::manager::ShareSubShareSub,
};
use common_base::log::{error, info};
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use protocol::{
    mqtt::{
        Disconnect, DisconnectProperties, MQTTPacket, PubComp, PubCompProperties, PubRec,
        PubRecProperties, Publish, PublishProperties, QoS, SubscribeProperties,
    },
    mqttv5::codec::Mqtt5Codec,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{
        broadcast,
        mpsc::{self, Receiver, Sender},
    },
    time::sleep,
};
use tokio_util::codec::Framed;

#[derive(Clone)]
pub struct SubscribeShareFollower {
    // (client_id, Sender<bool>)
    pub follower_sub_thread: DashMap<String, Sender<bool>>,
    pub subscribe_manager: Arc<SubscribeManager>,
    response_queue_sx4: broadcast::Sender<ResponsePackage>,
    response_queue_sx5: broadcast::Sender<ResponsePackage>,
    metadata_cache: Arc<MetadataCacheManager>,
}

impl SubscribeShareFollower {
    pub fn new(
        subscribe_manager: Arc<SubscribeManager>,
        response_queue_sx4: broadcast::Sender<ResponsePackage>,
        response_queue_sx5: broadcast::Sender<ResponsePackage>,
        metadata_cache: Arc<MetadataCacheManager>,
    ) -> Self {
        return SubscribeShareFollower {
            follower_sub_thread: DashMap::with_capacity(128),
            subscribe_manager,
            response_queue_sx4,
            response_queue_sx5,
            metadata_cache,
        };
    }

    pub async fn start(&self) {
        loop {
            for (client_id, share_sub) in self.subscribe_manager.share_follower_subscribe.clone() {
                let metadata_cache = self.metadata_cache.clone();
                let sub_manager = self.subscribe_manager.clone();
                let response_queue_sx4 = self.response_queue_sx4.clone();
                let response_queue_sx5 = self.response_queue_sx5.clone();
                let (sx, rx) = mpsc::channel(1);
                self.follower_sub_thread.insert(client_id.clone(), sx);

                tokio::spawn(async move {
                    if share_sub.protocol == MQTTProtocol::MQTT4 {
                        rewrite_sub_mqtt4(share_sub, rx).await;
                    } else if share_sub.protocol == MQTTProtocol::MQTT5 {
                        rewrite_sub_mqtt5(
                            metadata_cache,
                            sub_manager,
                            share_sub,
                            rx,
                            response_queue_sx4,
                            response_queue_sx5,
                        )
                        .await;
                    }
                });
            }
            sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn stop_client(&self, client_id: String) {
        if let Some(sx) = self.follower_sub_thread.get(&client_id) {
            match sx.send(true).await {
                Ok(_) => {}
                Err(_) => {}
            }
        }
    }

    pub async fn transport_leader(&self) {}
}

async fn rewrite_sub_mqtt4(share_sub: ShareSubShareSub, mut rx: Receiver<bool>) {
    //todo MQTT 4 does not currently support shared subscriptions
}

pub fn build_rewrite_subscribe_pkg(rewrite_sub: ShareSubShareSub) -> MQTTPacket {
    let subscribe = rewrite_sub.subscribe.clone();
    let mut subscribe_properties = SubscribeProperties::default();
    let mut user_properties = Vec::new();
    user_properties.push(share_sub_rewrite_publish_flag());
    subscribe_properties.user_properties = user_properties;
    subscribe_properties.subscription_identifier = Some(rewrite_sub.identifier_id as usize);
    return MQTTPacket::Subscribe(subscribe, Some(subscribe_properties));
}

async fn rewrite_sub_mqtt5(
    metadata_cache: Arc<MetadataCacheManager>,
    subscribe_manager: Arc<SubscribeManager>,
    share_sub: ShareSubShareSub,
    mut rx: Receiver<bool>,
    response_queue_sx4: broadcast::Sender<ResponsePackage>,
    response_queue_sx5: broadcast::Sender<ResponsePackage>,
) {
    let socket = TcpStream::connect(share_sub.leader_addr.clone())
        .await
        .unwrap();
    let mut stream: Framed<TcpStream, Mqtt5Codec> = Framed::new(socket, Mqtt5Codec::new());
    let packet = build_rewrite_subscribe_pkg(share_sub.clone());
    let _ = stream.send(packet).await;
    loop {
        match rx.try_recv() {
            Ok(flag) => {
                if flag {
                    info(format!(
                        "Rewrite sub thread for client [{}] was stopped successfully",
                        share_sub.client_id
                    ));
                    //todo unsubscribe
                    break;
                }
            }
            Err(_) => {}
        }
        if let Some(data) = stream.next().await {
            match data {
                Ok(da) => match da {
                    MQTTPacket::Publish(publish, publish_properties) => {
                        packet_publish(
                            subscribe_manager.clone(),
                            metadata_cache.clone(),
                            share_sub.clone(),
                            publish,
                            publish_properties,
                            response_queue_sx4.clone(),
                            response_queue_sx5.clone(),
                        )
                        .await;
                    }

                    MQTTPacket::PubRec(pubrec, pubrec_properties) => {
                        if let Some(properties) = pubrec_properties.clone() {
                            if is_share_sub_rewrite_publish(properties.user_properties) {
                                packet_pubrec(pubrec, pubrec_properties).await;
                            }
                        }
                    }

                    MQTTPacket::PubComp(pubcomp, pubcomp_properties) => {
                        if let Some(properties) = pubcomp_properties.clone() {
                            if is_share_sub_rewrite_publish(properties.user_properties) {
                                packet_pubcomp(pubcomp, pubcomp_properties).await;
                            }
                        }
                    }

                    MQTTPacket::Disconnect(disconnect, disconnect_properties) => {
                        packet_distinct(disconnect, disconnect_properties).await;
                        break;
                    }
                    _ => {
                        error("Rewrite subscription thread cannot recognize the currently returned package".to_string());
                    }
                },
                Err(e) => error(e.to_string()),
            }
        }
        match rx.try_recv() {
            Ok(flag) => {
                if flag {
                    break;
                }
            }
            Err(_) => {}
        }
    }
}

async fn packet_publish(
    subscribe_manager: Arc<SubscribeManager>,
    metadata_cache: Arc<MetadataCacheManager>,
    share_sub: ShareSubShareSub,
    publish: Publish,
    publish_properties: Option<PublishProperties>,
    response_queue_sx4: broadcast::Sender<ResponsePackage>,
    response_queue_sx5: broadcast::Sender<ResponsePackage>,
) {
    if let Some(properties) = publish_properties {
        if is_share_sub_rewrite_publish(properties.user_properties) {
            for iden_id in properties.subscription_identifiers {
                let client_id = if let Some(client_id) =
                    subscribe_manager.share_follower_identifier_id.get(&iden_id)
                {
                    client_id.clone()
                } else {
                    continue;
                };

                let connect_id = if let Some(sess) = metadata_cache.session_info.get(&client_id) {
                    if let Some(conn_id) = sess.connection_id {
                        conn_id
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                let mut sub_id = Vec::new();
                if let Some(sub_properties) = share_sub.subscribe_properties.clone() {
                    if let Some(id) = sub_properties.subscription_identifier {
                        sub_id.push(id);
                    }
                }

                let client_pub = Publish {
                    dup: false,
                    qos: publish.qos,
                    pkid: share_sub.subscribe.packet_identifier,
                    retain: false,
                    topic: publish.topic.clone(),
                    payload: publish.payload.clone(),
                };

                // If it is a shared subscription, it will be identified with the push message
                let mut user_properteis = Vec::new();
                user_properteis.push(share_sub_rewrite_publish_flag());

                let properties = PublishProperties {
                    payload_format_indicator: None,
                    message_expiry_interval: None,
                    topic_alias: None,
                    response_topic: None,
                    correlation_data: None,
                    user_properties: user_properteis,
                    subscription_identifiers: sub_id.clone(),
                    content_type: None,
                };

                let resp = ResponsePackage {
                    connection_id: connect_id,
                    packet: MQTTPacket::Publish(client_pub, Some(properties)),
                };
                match publish.qos {
                    QoS::AtLeastOnce => {

                        // stream.send(item);
                    }
                    QoS::ExactlyOnce => {}
                    QoS::AtMostOnce => {
                        publish_to_client(
                            share_sub.protocol.clone(),
                            resp,
                            response_queue_sx4.clone(),
                            response_queue_sx5.clone(),
                        )
                        .await;
                    }
                }
            }
        }
    }
}

async fn packet_pubrec(publish: PubRec, pubrec_properties: Option<PubRecProperties>) {}

async fn packet_pubcomp(publish: PubComp, pubcomp_properties: Option<PubCompProperties>) {}

async fn packet_distinct(publish: Disconnect, disconnect_properties: Option<DisconnectProperties>) {
}
#[cfg(test)]
mod tests {}
