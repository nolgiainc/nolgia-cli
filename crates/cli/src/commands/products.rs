use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use nolgia_client::types::{ImportProductRequest, Product};
use uuid::Uuid;

use crate::output::{OutputFormat, print_json};

use super::CommandContext;

#[derive(Subcommand, Debug)]
pub enum ProductsCommand {
    /// List your products
    List,
    /// Import a product from a public store link
    Import(ImportProductArgs),
    /// Fetch a product with fresh signed reference URLs
    Get(GetProductArgs),
    /// Delete a product
    Delete(DeleteProductArgs),
}

#[derive(Args, Debug)]
pub struct ImportProductArgs {
    pub url: String,
    /// File imported images into a project
    #[arg(long, value_name = "UUID")]
    pub project_id: Option<Uuid>,
}

#[derive(Args, Debug)]
pub struct GetProductArgs {
    pub product_id: Uuid,
}

#[derive(Args, Debug)]
pub struct DeleteProductArgs {
    pub product_id: Uuid,
}

pub async fn run(command: ProductsCommand, ctx: &CommandContext) -> Result<()> {
    match command {
        ProductsCommand::List => list(ctx).await,
        ProductsCommand::Import(args) => import(args, ctx).await,
        ProductsCommand::Get(args) => get(args, ctx).await,
        ProductsCommand::Delete(args) => delete(args, ctx).await,
    }
}

async fn list(ctx: &CommandContext) -> Result<()> {
    let list = ctx
        .client()
        .list_products()
        .send()
        .await
        .context("listing products")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &list),
        OutputFormat::Text => {
            for product in list.products {
                print_product_line(&product);
            }
            Ok(())
        }
    }
}

async fn import(args: ImportProductArgs, ctx: &CommandContext) -> Result<()> {
    let body = ImportProductRequest {
        url: args.url,
        project_id: args.project_id,
    };
    // Import is the one product call whose failures carry a reason the caller
    // has to read: a page the fetcher refused (422), an address it would not
    // resolve to, or the per-account rate limit (429). `api_error` surfaces the
    // server's RFC 7807 `detail` verbatim instead of progenitor's debug dump.
    let result = match ctx.client().import_product().body(body).send().await {
        Ok(result) => result.into_inner(),
        Err(err) => return Err(super::api_error(err, "importing product").await),
    };
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &result),
        OutputFormat::Text => {
            print_product_line(&result.product);
            println!(
                "imported {} of {} images",
                result.images_imported, result.images_found
            );
            Ok(())
        }
    }
}

async fn get(args: GetProductArgs, ctx: &CommandContext) -> Result<()> {
    let product = ctx
        .client()
        .get_product()
        .id(args.product_id)
        .send()
        .await
        .context("fetching product")?
        .into_inner();
    match ctx.format() {
        OutputFormat::Json => print_json(ctx.output(), &product),
        OutputFormat::Text => {
            print_product_line(&product);
            if !product.source_url.is_empty() {
                println!("{}", product.source_url.as_str());
            }
            if !product.canonical_description.is_empty() {
                println!("{}", product.canonical_description.as_str());
            }
            let primary_id = product
                .primary_image_asset_id
                .or_else(|| product.images.first().map(|image| image.id));
            for image in &product.images {
                let primary = if Some(image.id) == primary_id {
                    " (primary)"
                } else {
                    ""
                };
                println!("{} {}{primary}", image.id, image.signed_url);
            }
            Ok(())
        }
    }
}

async fn delete(args: DeleteProductArgs, ctx: &CommandContext) -> Result<()> {
    ctx.client()
        .delete_product()
        .id(args.product_id)
        .send()
        .await
        .context("deleting product")?;
    match ctx.format() {
        OutputFormat::Json => print_json(
            ctx.output(),
            &serde_json::json!({ "deleted": args.product_id }),
        ),
        OutputFormat::Text => {
            println!("deleted {}", args.product_id);
            Ok(())
        }
    }
}

fn print_product_line(product: &Product) {
    print!("{} {} (", product.id, product.name.as_str());
    for detail in [product.brand.as_str(), product.price.as_str()] {
        if !detail.is_empty() {
            print!("{detail}, ");
        }
    }
    println!(
        "{} image{})",
        product.images.len(),
        if product.images.len() == 1 { "" } else { "s" }
    );
}
