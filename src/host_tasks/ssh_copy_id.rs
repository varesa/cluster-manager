mod podspec;

use serde_json::json;
use k8s_openapi::api::core::v1::{Node, Pod};
use kube::{
    api::{Api, ListParams, PatchParams, PostParams},
    Client
};

use crate::errors::Error;
use crate::ssh_copy_id::podspec::make_pod;
use crate::utils::wait_for_pod;

const ANNOTATION_SSH: &str = "cluster-manager/ssh";

async fn add_ssh_key(client: Client, namespace: String, node: &Node) -> Result<(), Error> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace.as_ref());
    let nodes: Api<Node> = Api::all(client.clone());
    let node_name = node.metadata.name.as_ref().unwrap().clone();

    let pod = make_pod(&node_name);
    let pod_name = pod.metadata.name.as_ref().unwrap();

    pods.create(&PostParams::default(), &pod).await?;
    wait_for_pod(&pods, &pod_name).await?;

    nodes.patch(
        &node_name,
        &PatchParams::default(),
        serde_json::to_vec(
            &json!({ "metadata": { "annotations": { "cluster-manager/ssh": "true" } } })
        )?
    ).await?;
    Ok(())
}

async fn prepare_host(client: Client, namespace: String, node: Node) -> Result<(), Error>{
    let name = node.metadata.name.as_ref().unwrap();
    println!("Working on {}", name);

    let log = |msg: &str| {
        println!("{}: {}", name, msg);
    };

    if node.metadata.annotations.as_ref().unwrap().contains_key(ANNOTATION_SSH) {
        log(format!("{} annotation present, skipping", ANNOTATION_SSH).as_ref());
        return Ok(());
    }

    log("No SSH access prepared (no annotation)");
    add_ssh_key(client.clone(), namespace, &node).await?;

    Ok(())
}


pub async fn run(client: Client, namespace: String) -> Result<(), Error> {
    let nodes: Api<Node> = Api::all(client.clone());

    let mut tasks = Vec::new();
    for node in nodes.list(&ListParams::default()).await? {
        let c = client.clone();
        let n = namespace.clone();
        tasks.push(tokio::spawn(async move {
            prepare_host(c, n, node).await?;
            Ok::<(), Error>(())
        }));
    }

    let mut errors = Vec::new();
    for result in futures::future::join_all(tasks).await {
        match result {
            // Unhandled error
            Err(e) => panic!(e),
            // Task completed, with or without errors
            Ok(result) => match result {
                Ok(_) => println!("Task completed succesfully"),
                Err(e) => errors.push(e),
            }
        }
    }

    if errors.len() > 0 {
        Err(Error::MultipleErrors(errors))
    } else {
        Ok(())
    }
}
