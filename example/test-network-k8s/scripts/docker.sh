#!/bin/bash
# Copyright 2023 RobustMQ Team
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

function docker_build() {
    local mqtt_server_tag=${MQTT_SERVER_IMAGE_NAME}:${IMAGE_VERSION}
    #local journal_server_tag=${JOURNAL_SERVER_IMAGE_NAME}:${IMAGE_VERSION}
    local placement_center_tag=${PLACEMENT_CENTER_IMAGE_NAME}:${IMAGE_VERSION}
    cd ../../
    docker build --target placement-center -t ${placement_center_tag} .
    docker build --target mqtt-server -t ${mqtt_server_tag} .
    #docker build --target journal-server -t ${journal_server_tag} .
}
