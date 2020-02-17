/*
 * Copyright 2020 Damian Peckett <damian@pecke.tt>
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::error::*;
use crate::kubernetes::common::{build_label_selector, get_running_pod_ips};
use crate::kubernetes::{KubernetesObject, KubernetesObjectTrait, KubernetesResourceTrait};
use crate::resource::ANNOTATION_BASE;
use async_trait::async_trait;
use chrono::prelude::*;
use k8s_openapi::api::apps::v1::StatefulSetSpec;
use kube::api::Api;
use kube::api::{ListParams, ObjectMeta, PatchParams};
use kube::client::APIClient;
use serde_json::json;
use snafu::{OptionExt, ResultExt};
use std::collections::BTreeMap;

/// Kubernetes StatefulSet resource kind related functions.
pub struct KubernetesStatefulSetResource {
    kube_config: kube::config::Configuration,
    namespace: String,
    label_selector: String,
}

impl KubernetesStatefulSetResource {
    pub fn new(
        kube_config: kube::config::Configuration,
        namespace: &str,
        match_labels: &BTreeMap<String, String>,
    ) -> Self {
        Self {
            kube_config,
            namespace: namespace.into(),
            label_selector: build_label_selector(match_labels),
        }
    }
}

#[async_trait]
impl KubernetesResourceTrait for KubernetesStatefulSetResource {
    async fn list(&self) -> Result<Vec<KubernetesObject>, Error> {
        let kube_client = APIClient::new(self.kube_config.clone());
        // Retrieve the list of StatefulSet objects matching the label selector.
        let statefulsets = Api::v1StatefulSet(kube_client)
            .within(&self.namespace)
            .list(&ListParams {
                label_selector: Some(self.label_selector.clone()),
                ..Default::default()
            })
            .await
            .context(Kube {})?;
        let mut objects: Vec<KubernetesObject> = Vec::new();
        for statefulset in statefulsets {
            objects.push(KubernetesObject::StatefulSet(
                KubernetesStatefulSetObject::new(
                    self.kube_config.clone(),
                    &self.namespace,
                    &statefulset.metadata,
                    &statefulset.spec,
                ),
            ))
        }
        Ok(objects)
    }
}

/// Kubernetes StatefulSet related functions.
pub struct KubernetesStatefulSetObject {
    kube_config: kube::config::Configuration,
    namespace: String,
    metadata: ObjectMeta,
    spec: StatefulSetSpec,
}

impl KubernetesStatefulSetObject {
    pub fn new(
        kube_config: kube::config::Configuration,
        namespace: &str,
        metadata: &ObjectMeta,
        spec: &StatefulSetSpec,
    ) -> Self {
        Self {
            kube_config,
            namespace: namespace.into(),
            metadata: metadata.clone(),
            spec: spec.clone(),
        }
    }
}

#[async_trait]
impl KubernetesObjectTrait for KubernetesStatefulSetObject {
    fn namespace_and_name(&self) -> (String, String) {
        (self.namespace.clone(), self.metadata.name.clone())
    }

    async fn last_modified(&self) -> Result<Option<DateTime<Utc>>, Error> {
        Ok(
            // Retrieve the last modified timestamp from the StatefulSet's annotations.
            if let Some(last_modified_timestamp) = self
                .metadata
                .annotations
                .get(&format!("{}/last_modified", ANNOTATION_BASE))
            {
                Some(DateTime::from_utc(
                    DateTime::<FixedOffset>::parse_from_rfc3339(last_modified_timestamp)
                        .unwrap()
                        .naive_utc(),
                    Utc,
                ))
            } else {
                None
            },
        )
    }

    async fn replicas(&self) -> Result<u32, Error> {
        self.spec
            .replicas
            .context(KubeSpec {})
            .map(|replicas| replicas as u32)
    }

    async fn pod_ips(&self) -> Result<Vec<String>, Error> {
        let labels = self
            .spec
            .template
            .metadata
            .as_ref()
            .context(KubeSpec {})?
            .labels
            .as_ref()
            .context(KubeSpec {})?;
        let kube_client = APIClient::new(self.kube_config.clone());
        get_running_pod_ips(kube_client, &self.namespace, labels).await
    }

    async fn scale(&self, replicas: u32) -> Result<(), Error> {
        let utc_now: DateTime<Utc> = Utc::now();
        let patch = json!({
            "metadata": {
                "annotations": {
                    format!("{}/last_modified", ANNOTATION_BASE): utc_now.to_rfc3339()
                }
            },
            "spec": {
                "replicas": replicas
            }
        });
        // Patch (update) the StatefulSet object.
        let patch_params = PatchParams::default();
        let kube_client = APIClient::new(self.kube_config.clone());
        Api::v1StatefulSet(kube_client)
            .within(&self.namespace)
            .patch(
                &self.metadata.name,
                &patch_params,
                serde_json::to_vec(&patch).context(JsonSerialization {})?,
            )
            .await
            .context(Kube {})?;
        Ok(())
    }
}
