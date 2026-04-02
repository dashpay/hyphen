//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Script index SQL operations.

use crate::error::DaemonError;

pub async fn ensure_output_history_partition(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(&crate::sql::partition::ensure_script_output_history_partition_sql(height))
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("ensure script history partition: {e}")))?;
  Ok(())
}

pub async fn restore_output_addresses(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE tx_outputs AS o
        SET address = h.prev_address,
            address_type = h.prev_address_type
       FROM script_output_history AS h
      WHERE h.height = $1
        AND o.txid = h.txid
        AND o.vout_index = h.vout_index",
  )
  .bind(height as i32)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("restore output addr at {height}: {e}")))?;

  sqlx::query("DELETE FROM script_output_history WHERE height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete script history at {height}: {e}")))?;
  Ok(())
}
