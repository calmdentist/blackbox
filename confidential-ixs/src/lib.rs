use arcis::prelude::*;
use crypto::*;

arcis_linker!();

#[derive(ArcisType, Copy, Clone, ArcisEncryptable)]
pub struct Mapping {
    pub pubkeys: Vec<PublicKey>,
    pub balances: Vec<u64>,
}

pub struct Transfer {
    pub from: PublicKey,
    pub to: PublicKey,
    pub amount: u64,
}

#[confidential]
pub fn init_mapping(mapping_nonce: u128) -> [Ciphertext; 2] {
    let cipher = RescueCipher::new_for_mxe();
    let mapping = Mapping {
        pubkeys: vec![],
        balances: vec![],
    };
    cipher.encrypt::<1, Mapping>(mapping, mapping_nonce)
}

#[confidential]
pub fn deposit(
    to: PublicKey,
    deposit_amount: u64,
    mapping: [Ciphertext; 2],
    mapping_nonce: u128,
    nonce: u128,
) -> [Ciphertext; 2] {
    let cipher = RescueCipher::new_for_mxe();
    
    // Decrypt the mapping
    let mut mapping_data = cipher.decrypt::<Mapping>(mapping, mapping_nonce);
    
    // Search for the recipient's public key in the mapping
    let mut found = false;
    for i in 0..mapping_data.pubkeys.len() {
        if mapping_data.pubkeys[i] == to {
            // Found the recipient, update their balance
            mapping_data.balances[i] += deposit_amount;
            found = true;
            break;
        }
    }
    
    // If recipient not found, add a new entry
    if !found {
        mapping_data.pubkeys.push(to);
        mapping_data.balances.push(deposit_amount);
    }
    
    // Re-encrypt the updated mapping
    cipher.encrypt::<1, Mapping>(mapping_data, nonce)
}

#[confidential]
pub fn transfer(
    mapping: [Ciphertext; 2],
    mapping_nonce: u128,
    from: PublicKey,
    to: Ciphertext,
    transfer_amount: Ciphertext,
    nonce: u128,
) -> [Ciphertext; 2] {
    let cipher = RescueCipher::new_for_mxe();
    
    // Decrypt the mapping
    let mut mapping_data = cipher.decrypt::<Mapping>(mapping, mapping_nonce);
    
    // Find sender and recipient indices
    let mut sender_idx = None;
    let mut recipient_idx = None;

    let to_decrypted = cipher.decrypt::<PublicKey>(to, nonce);
    let transfer_amount_decrypted = cipher.decrypt::<u64>(transfer_amount, nonce);
    
    for i in 0..mapping_data.pubkeys.len() {
        if mapping_data.pubkeys[i] == from {
            sender_idx = Some(i);
        } else if mapping_data.pubkeys[i] == to_decrypted {
            recipient_idx = Some(i);
        }
        
        // If we found both, we can break early
        if sender_idx.is_some() && recipient_idx.is_some() {
            break;
        }
    }
    
    // Verify sender exists and transfer funds if balance is sufficient
    if let Some(idx) = sender_idx {
        if mapping_data.balances[idx] >= transfer_amount {
            // Subtract from sender
            mapping_data.balances[idx] -= transfer_amount_decrypted;
            
            // Add to recipient if they exist, otherwise create new entry
            if let Some(idx) = recipient_idx {
                mapping_data.balances[idx] += transfer_amount_decrypted;
            } else {
                mapping_data.pubkeys.push(to_decrypted);
                mapping_data.balances.push(transfer_amount_decrypted);
            }
        }
    }
    
    // Re-encrypt the updated mapping
    cipher.encrypt::<1, Mapping>(mapping_data, nonce)
}

#[confidential]
pub fn withdraw(
    mapping: [Ciphertext; 2],
    mapping_nonce: u128,
    from: PublicKey,
    withdraw_amount: u64,
    nonce: u128,
) -> ([Ciphertext; 2], bool) {
    let cipher = RescueCipher::new_for_mxe();
    
    // Decrypt the mapping
    let mut mapping_data = cipher.decrypt::<Mapping>(mapping, mapping_nonce);
    
    // Find the user's account
    let mut user_idx = None;
    
    for i in 0..mapping_data.pubkeys.len() {
        if mapping_data.pubkeys[i] == from {
            user_idx = Some(i);
            break;
        }
    }
    
    // If user exists and has sufficient balance, process the withdrawal
    if let Some(idx) = user_idx {
        if mapping_data.balances[idx] >= withdraw_amount {
            // Subtract the withdrawal amount from the user's balance
            mapping_data.balances[idx] -= withdraw_amount;

            // Re-encrypt the updated mapping, return true since withdrawal succeeded
            return (cipher.encrypt::<1, Mapping>(mapping_data, nonce), true);
        }
    }
    
    // Re-encrypt the updated mapping, return false since withdrawal failed
    (cipher.encrypt::<1, Mapping>(mapping_data, nonce), false)
}
