//! `items_rekey`: giving the Items an Interface already keeps the keys and the lines the provider of its language
//! finds in the code it keeps. It is how an Interface put before its Items were found by a provider (or whose keys
//! were converted from signatures) comes to the provider's keys, with the ids, and so the citations, kept.

use std::collections::BTreeMap;

use super::change_set::items_error;
use super::ports::external::ExternalExecutor;
use super::ports::{AssetKind, IdSource, Store};
use super::{App, AppError, context_id, input};
use crate::domain::service::rekey::pair;
use crate::domain::vo::id::{InterfaceId, ItemId};
use crate::domain::vo::item_key::ItemKey;
use crate::domain::vo::line_range::LineRange;

/// An Item that was given another key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChanged {
    /// The key it had.
    pub from: String,
    /// The key it has.
    pub to: String,
}

/// An Item whose revision went up, because the lines the provider found for it declare something else than the
/// lines it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRevised {
    /// The key it has now.
    pub key: String,
    /// The revision it had.
    pub from: u64,
    /// The revision it has.
    pub to: u64,
}

/// What `items_rekey` did to an Interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rekeyed {
    /// The Items that were given another key.
    pub rekeyed: Vec<KeyChanged>,
    /// The Items whose revision went up (by the new lines they have, not by the new key).
    pub revised: Vec<ItemRevised>,
    /// The keys of the Items no declaration was paired with: they are as they were. Either no declaration the
    /// provider found is on their lines, or they and the declarations share lines with others (see
    /// [`crate::domain::service::rekey::pair`]).
    pub unmatched: Vec<String>,
    /// The revision of the Interface after the change.
    pub revision: u64,
}

impl<S: Store, G: IdSource, E: ExternalExecutor> App<S, G, E> {
    /// Gives the Items of the Interface named `interface` of the Context `context` the key and the lines the
    /// provider of its language finds in **the code the Interface keeps** (the working tree is not read). An Item
    /// and a declaration are paired when their lines overlap and each overlaps no other of the other side; an Item
    /// that is not paired stays as it is and is listed in [`Rekeyed::unmatched`].
    ///
    /// The id of an Item is kept, so its citations are. Its revision follows the rule of a put: a new key alone
    /// leaves it, and lines that declare something else take it to the next one (and the Interface with it); those
    /// are listed in [`Rekeyed::revised`]. A key that two Items would have, a provider that is not there or fails
    /// are refused with nothing changed. The change is kept as the store stood when the use case began, so a store
    /// that has moved since refuses it. It is no change set and has no footprint.
    pub async fn items_rekey(
        &mut self,
        context: &str,
        interface: &str,
    ) -> Result<Rekeyed, AppError> {
        let at = self.store.revision().await?;
        let mut p = self.project().await?;
        let ctx = context_id(&p, context)?;
        let id = InterfaceId::from_uuid(
            self.asset(ctx, context, AssetKind::Interface, interface)
                .await?,
        );
        let Some(old) = p.context(ctx).and_then(|c| c.interface(id)).cloned() else {
            return Err(AppError::AssetRetired {
                context: context.to_string(),
                kind: AssetKind::Interface.word(),
                name: interface.to_string(),
            });
        };
        let lang = self
            .items
            .language(context, interface)
            .map_err(|e| items_error(context, interface, None, e))?;
        let found = self
            .items
            .items(&lang, old.source().as_str())
            .await
            .map_err(|e| items_error(context, interface, Some(&lang), e))?;
        let ranges = found
            .iter()
            .map(|f| LineRange::new(f.lines.0, f.lines.1))
            .collect::<Result<Vec<_>, _>>()
            .map_err(input)?;
        let kept: Vec<LineRange> = old.items().iter().map(|i| i.lines()).collect();
        let mut map: BTreeMap<ItemId, (ItemKey, LineRange)> = BTreeMap::new();
        let mut unmatched = Vec::new();
        for (item, paired) in old.items().iter().zip(pair(&kept, &ranges)) {
            match paired {
                Some(n) => {
                    let key = ItemKey::try_from(found[n].key.as_str()).map_err(input)?;
                    map.insert(item.id(), (key, ranges[n]));
                }
                None => unmatched.push(item.key().to_string()),
            }
        }
        p.rekey_interface(ctx, id, &map)?;
        let now = p
            .context(ctx)
            .and_then(|c| c.interface(id))
            .ok_or_else(|| input("the interface is gone after its items were given keys"))?;
        let mut rekeyed = Vec::new();
        let mut revised = Vec::new();
        for (before, after) in old.items().iter().zip(now.items()) {
            if before.key() != after.key() {
                rekeyed.push(KeyChanged {
                    from: before.key().to_string(),
                    to: after.key().to_string(),
                });
            }
            if before.revision() != after.revision() {
                revised.push(ItemRevised {
                    key: after.key().to_string(),
                    from: before.revision().number(),
                    to: after.revision().number(),
                });
            }
        }
        let revision = now.revision().number();
        if *now != old {
            self.commit_project(at, p).await?;
        }
        Ok(Rekeyed {
            rekeyed,
            revised,
            unmatched,
            revision,
        })
    }
}
