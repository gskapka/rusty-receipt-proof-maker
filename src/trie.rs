use crate::simple_logger;
use ethereum_types::H256;
use crate::errors::AppError;
use crate::utils::{
    convert_bytes_to_h256,
    convert_h256_to_bytes,
};
use crate::trie_nodes::{
    Node,
    rlp_decode_node,
    get_node_from_database,
};
use crate::nibble_utils::{
    Nibbles,
    get_nibble_at_index,
    get_length_in_nibbles,
    split_at_first_nibble,
    get_nibbles_from_bytes,
    convert_nibble_to_usize,
    get_common_prefix_nibbles,
};
use crate::constants::{
    EMPTY_NIBBLES,
    HASHED_NULL_NODE,
    LEAF_NODE_STRING,
    EMPTY_NODE_STRING,
    BRANCH_NODE_STRING,
    EXTENSION_NODE_STRING,
};
use crate::get_database::{
    get_new_database,
    put_thing_in_database,
    remove_thing_from_database,
};
use crate::types::{
    Bytes,
    Result,
    Database,
    NodeStack,
};

#[derive(Clone)]
pub struct Trie {
    pub root: H256,
    pub database: Database,
}

impl Trie {
    pub fn get_new_trie() -> Result<Trie> {
        Ok(
            Trie {
                root: HASHED_NULL_NODE,
                database: get_new_database()?
            }
        )
    }

    pub fn put(self, key: Nibbles, value: Bytes) -> Result<Self> {
        trace!("Putting new value in trie under path: {:?}", key);
        match self.root == HASHED_NULL_NODE {
            true => {
                trace!("Trie empty ∴ creating new leaf node...");
                Node::get_new_leaf_node(key.clone(), value)
                    .and_then(|leaf|
                        self.update_trie_database(
                            vec![leaf],
                            Vec::new(),
                        )
                    )
            },
            false => {
                trace!("Trie not-empty ∴ finding nearest node to key...");
                Trie::find(self, key)
                    .and_then(|(self_, target_key, found_stack, remaining_key)|
                        self_.process_found_node_stack(
                            target_key,
                            found_stack,
                            remaining_key,
                            value,
                        )
                    )
                    .and_then(|(self_, target_key, old_stack, new_stack, stack_to_delete)|
                        self_.update_stale_nodes(
                            target_key,
                            old_stack,
                            new_stack,
                            stack_to_delete,
                        )
                    )
                    .and_then(|(self_, target_key, old_stack, new_stack, stack_to_delete)|
                        self_.update_trie_database(
                            new_stack,
                            stack_to_delete,
                        )
                    )
            }
        }
    }

    fn process_found_node_stack(
        self,
        target_key: Nibbles,
        mut found_stack: NodeStack,
        remaining_key: Nibbles,
        value: Bytes,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        match found_stack.pop() {
            Some(node) => {
                match node.get_type() {
                    "leaf" => self.process_from_leaf_node(
                        target_key,
                        node,
                        found_stack,
                        remaining_key,
                        value,
                    ),
                    "branch" => self.process_from_branch_node(
                        target_key,
                        node,
                        found_stack,
                        remaining_key,
                        value,
                    ),
                    "extension" => self.process_from_extension_node(
                        target_key,
                        node,
                        found_stack,
                        remaining_key,
                        value,
                    ),
                    _ => Err(AppError::Custom(
                        "✘ Node type not recognized!".to_string()
                    ))
                }
            },
            None =>  Err(AppError::Custom(
                "✘ Cannot process node stack: It's empty!".to_string()
            )),
        }
    }
    /**
     *
     * Processing from an extension node has the following two cases:
     *
     * 1) No common-prefix between extension node's path and remaining key.
     * 2) A common-prefix between extension node's path and remaining key.
     *
     * In the first case we split the nodes at the first nibbles and create a
     * branch to point at the original extension - which is first updated with
     * a path one nibble shorter - and a leaf which is created from the
     * remaining key minus its first nibble and the value to be sorted.
     *
     * In the second, much the same as above occurs except the split happens
     * somewhere other than the first nibble. In this case, a new extension is
     * created to consume the common prefix, which then points at the branch
     * and it's children per case 1).
     *
     */
    fn process_from_extension_node(
        self,
        target_key: Nibbles,
        current_ext_node: Node,
        mut found_stack: NodeStack,
        remaining_key: Nibbles,
        value: Bytes,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        trace!("Processing from extension node...");
        let mut new_stack: NodeStack = Vec::new();
        get_common_prefix_nibbles(remaining_key, current_ext_node.get_key())
            .and_then(|(common_prefix, key_remainder, node_key_remainder)| {
                match common_prefix.len() {
                    0 => split_at_first_nibble(&node_key_remainder)
                        .and_then(|(ext_first_nibble, ext_nibbles)| {
                            trace!(
                                "No common prefix ∴ augmenting existing {}",
                                "ext to: branch -> ext & leaf"
                            );
                            let (
                                key_remainder_first_nibble,
                                key_remainder_nibbles
                            ) = split_at_first_nibble(&key_remainder)?;
                            let new_leaf = Node::get_new_leaf_node(
                                key_remainder_nibbles,
                                value
                            )?;
                            let new_ext = Node::get_new_extension_node(
                                ext_nibbles,
                                current_ext_node.get_value()?
                            )?;
                            let branch = Node::get_new_branch_node(None)?;
                            let updated_branch_1 = branch.update_branch_at_index(
                                Some(convert_h256_to_bytes(new_ext.get_hash()?)),
                                convert_nibble_to_usize(ext_first_nibble)
                            )?;
                            let new_branch = updated_branch_1.update_branch_at_index(
                                Some(convert_h256_to_bytes(new_leaf.get_hash()?)),
                                convert_nibble_to_usize(key_remainder_first_nibble)
                            )?;
                            new_stack.push(new_leaf);
                            new_stack.push(new_ext);
                            new_stack.push(new_branch);
                            Ok((
                                self,
                                target_key,
                                found_stack,
                                new_stack,
                                Vec::new()
                            ))
                        }),
                    _ => {
                        trace!(
                            "Common prefix ∴ augmenting existing ext to: {}",
                            "ext -> branch -> ext & leaf"
                        );
                        let (
                            key_remainder_first_nibble,
                            key_remainder_nibbles
                        ) = split_at_first_nibble(&key_remainder)?;
                        let (
                            node_key_remainder_first_nibble,
                            node_key_remainder_nibbles
                        ) = split_at_first_nibble(&node_key_remainder)?;
                        let ext_below_branch = Node::get_new_extension_node(
                            node_key_remainder_nibbles,
                            current_ext_node.get_value()?
                        )?;
                        let new_leaf = Node::get_new_leaf_node(
                            key_remainder_nibbles,
                            value
                        )?;
                        let empty_branch = Node::get_new_branch_node(None)?;
                        let updated_branch = empty_branch.update_branch_at_index(
                            Some(convert_h256_to_bytes(new_leaf.get_hash()?)),
                            convert_nibble_to_usize(key_remainder_first_nibble)
                        )?;
                        let final_branch = updated_branch.update_branch_at_index(
                            Some(convert_h256_to_bytes(ext_below_branch.get_hash()?)),
                            convert_nibble_to_usize(node_key_remainder_first_nibble)
                        )?;
                        let final_branch_hash = convert_h256_to_bytes(
                            final_branch.get_hash()?
                        );
                        let ext_above_branch = Node::get_new_extension_node(
                            common_prefix,
                            final_branch_hash
                        )?;
                        new_stack.push(new_leaf);
                        new_stack.push(ext_below_branch);
                        new_stack.push(final_branch);
                        new_stack.push(ext_above_branch);
                        Ok((
                            self,
                            target_key,
                            found_stack,
                            new_stack,
                            Vec::new()
                        ))
                    }
                }
            })
    }
    /**
     *
     * Processing from a leaf node considers the following cases:
     *
     * 1) No remaining target key.
     * 2) Some remaining key w/ a common prefix between it and the found leaf.
     * 3) Some remaining key w/ no common prefix between it and the found leaf.
     *
     * The first case is a full match, and so we simply update the value found
     * in the leaf node to the new value provided.
     *
     * The second case is a partial key match which requires a split at the
     * first nibble of the remaining key, creating a branch. The existing leaf's
     * path is then shortened by a single nibble, and a second leaf is created
     * from the remaining key (minus its first nibble) and the final value.
     * The hashes of those leaves are then updated in the previously created
     * branch.
     *
     * The third case is also a partial match, but which requires a split
     * somewhere other than the first nibble of the remaining key. This results
     * in the same branch & two new leaves (the original one w/ a now shorter
     * path) as in case two, as well as an extension node consuming the common
     * prefix and pointing to the branch at which the divergence occurs...
     *
     */
    fn process_from_leaf_node(
        self,
        target_key: Nibbles,
        current_leaf_node: Node,
        mut found_stack: NodeStack,
        remaining_key: Nibbles,
        value: Bytes,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        trace!("Processing from leaf node...");
        let mut new_stack: NodeStack = Vec::new();
        match remaining_key.len() {
            0 => Node::get_new_leaf_node(
                    current_leaf_node.get_key(),
                    value
                )
                    .and_then(|new_leaf| {
                        trace!("No key remaining ∴ creating new leaf node");
                        new_stack.push(new_leaf);
                        Ok((
                            self,
                            target_key,
                            found_stack,
                            new_stack,
                            Vec::new()
                        ))
                    }),
            _ => get_common_prefix_nibbles(remaining_key, current_leaf_node.get_key())
                .and_then(|(common_prefix, key_remainder, node_key_remainder)| {
                    match common_prefix.len() {
                        0 => split_at_first_nibble(&node_key_remainder)
                            .and_then(|(leaf_first_nibble, leaf_nibbles)| {
                                trace!(
                                    "No common prefix ∴ creating: {}",
                                    "branch -> 2 leaves"
                                );
                                let (
                                    key_remainder_first_nibble,
                                    key_remainder_nibbles
                                ) = split_at_first_nibble(&key_remainder)?;
                                let new_leaf_1 = Node::get_new_leaf_node(
                                    leaf_nibbles,
                                    current_leaf_node.get_value()?
                                )?;
                                let new_leaf_2 = Node::get_new_leaf_node(
                                    key_remainder_nibbles,
                                    value
                                )?;
                                let new_branch = Node::get_new_branch_node(None)?;
                                let updated_branch_1 = new_branch.update_branch_at_index(
                                    Some(convert_h256_to_bytes(new_leaf_1.get_hash()?)),
                                    convert_nibble_to_usize(leaf_first_nibble)
                                )?;
                                let updated_branch = updated_branch_1.update_branch_at_index(
                                    Some(convert_h256_to_bytes(new_leaf_2.get_hash()?)),
                                    convert_nibble_to_usize(key_remainder_first_nibble)
                                )?;
                                new_stack.push(updated_branch);
                                new_stack.push(new_leaf_1);
                                new_stack.push(new_leaf_2);
                                Ok((
                                    self,
                                    target_key,
                                    found_stack,
                                    new_stack,
                                    Vec::new()
                                ))
                            }),
                        _ => split_at_first_nibble(&node_key_remainder)
                            .and_then(|(leaf_first_nibble, leaf_nibbles)| {
                                trace!(
                                    "Common prefix ∴ creating: ext -> branch{}",
                                    " -> 2 leaves"
                                );
                                let (
                                    key_remainder_first_nibble,
                                    key_remainder_nibbles
                                ) = split_at_first_nibble(&key_remainder)?;
                                let new_leaf_1 = Node::get_new_leaf_node(
                                    leaf_nibbles,
                                    current_leaf_node.get_value()?
                                )?;
                                let new_leaf_2 = Node::get_new_leaf_node(
                                    key_remainder_nibbles,
                                    value
                                )?;
                                let new_branch = Node::get_new_branch_node(None)?;
                                let new_branch_hash = convert_h256_to_bytes(
                                    new_branch.get_hash()?
                                );
                                let updated_branch_1 = new_branch.update_branch_at_index(
                                    Some(convert_h256_to_bytes(new_leaf_1.get_hash()?)),
                                    convert_nibble_to_usize(leaf_first_nibble)
                                )?;
                                let updated_branch = updated_branch_1
                                    .update_branch_at_index(
                                    Some(convert_h256_to_bytes(new_leaf_2.get_hash()?)),
                                    convert_nibble_to_usize(key_remainder_first_nibble)
                                )?;
                                let new_extension = Node::get_new_extension_node(
                                    common_prefix,
                                    new_branch_hash
                                )?;
                                new_stack.push(new_extension);
                                new_stack.push(updated_branch);
                                new_stack.push(new_leaf_1);
                                new_stack.push(new_leaf_2);
                                Ok((
                                    self,
                                    target_key,
                                    found_stack,
                                    new_stack,
                                    Vec::new()
                                ))
                            })
                        }
                })
        }
    }
    /**
     *
     * Here we create a new leaf node from the remaining key minus its first
     * nibble. Next we get that nodes hash and add it to the current branch
     * node, at the index the first nibble we chopped off the remaining key
     * points to.
     *
     */
    fn process_from_branch_node(
        self,
        target_key: Nibbles,
        current_branch_node: Node,
        mut found_stack: NodeStack,
        remaining_key: Nibbles,
        value: Bytes,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        trace!("Processing from branch node...");
        split_at_first_nibble(&remaining_key)
            .and_then(|(first_nibble, remaining_nibbles)| {
                trace!("Creating new leaf & updating branch node...");
                let new_leaf = Node::get_new_leaf_node(
                    remaining_nibbles, value
                )?;
                let new_leaf_hash = convert_h256_to_bytes(new_leaf.get_hash()?);
                let updated_branch = current_branch_node.update_branch_at_index(
                    Some(new_leaf_hash),
                    convert_nibble_to_usize(first_nibble)
                )?;
                let mut new_stack: NodeStack = Vec::new();
                new_stack.push(updated_branch);
                new_stack.push(new_leaf);
                Ok(new_stack)
            })
            .and_then(|new_stack|
                Ok((
                    self,
                    target_key,
                    found_stack,
                    new_stack,
                    Vec::new()
                ))
            )
    }

    fn update_stale_nodes(
        self,
        target_key: Nibbles,
        mut old_stack: NodeStack,
        mut new_stack: NodeStack,
        mut stack_to_delete: NodeStack,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        match old_stack.pop() {
            Some(current_node) => match current_node.get_type() {
                "branch" => self.update_nodes_from_old_branch_node(
                    target_key,
                    current_node,
                    old_stack,
                    new_stack,
                    stack_to_delete,
                ),
                "extension" => self.update_nodes_from_old_extension_node(
                    target_key,
                    current_node,
                    old_stack,
                    new_stack,
                    stack_to_delete,
                ),
                _ => Err(AppError::Custom(
                    "✘ Error updating old nodes: Wrong node type!".to_string()
                ))
            },
            None => Ok((self, target_key, old_stack, new_stack, stack_to_delete))
        }
    }
    /**
     *
     * Here we take the old extension node and update the hash it contains to
     * the hash of the next node in the trie, which lives at the start of the
     * `new_nodes` stack. This new node is unshifted into the `new_node` stack.
     * The old extension node is then condemned to the `delete_stack` for later
     * deletion.
     *
     */
    fn update_nodes_from_old_extension_node(
        self,
        target_key: Nibbles,
        current_node: Node,
        mut old_stack: NodeStack,
        mut new_stack: NodeStack,
        mut stack_to_delete: NodeStack,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        trace!("Updating stale nodes from old extension node...");
        let target_node_hash = new_stack[0].get_hash()?;
        let updated_extension_node = Node::get_new_extension_node(
            current_node.get_key(),
            convert_h256_to_bytes(target_node_hash)
        )?;
        new_stack.insert(0, updated_extension_node);
        stack_to_delete.push(current_node);
        Ok((
            self,
            target_key,
            old_stack,
            new_stack,
            stack_to_delete,
        ))
    }
    /**
     *
     * Here we take the old branch node and update it to contain the next node
     * in line's hash, placed at the correct index in the branches. Which
     * latter is calculated by finding out how much of the target key so far
     * is accounted for in the new_node stack, and getting the nibble
     * immediately before that.
     *
     * This updated branch node is then unshifted into the `new_node` stack,
     * and the old branch node condemned to the `stack_to_delete` for later
     * deletion.
     *
     */
    fn update_nodes_from_old_branch_node(
        self,
        target_key: Nibbles,
        current_node: Node,
        mut old_stack: NodeStack,
        mut new_stack: NodeStack,
        mut stack_to_delete: NodeStack,
    ) -> Result<(Self, Nibbles, NodeStack, NodeStack, NodeStack)> {
        trace!("Updating stale nodes from old branch node...");
        let target_node_hash = new_stack[0].get_hash()?;
        let key_partial_length = get_key_length_accounted_for_in_stack(
            &new_stack
        );
        let nibble_index = target_key.len() - key_partial_length - 1;
        let byte = get_nibble_at_index(
            &target_key,
            nibble_index
        )?;
        let nibble = get_nibbles_from_bytes(vec![byte]);
        let branch_index = convert_nibble_to_usize(nibble);
        let updated_node = current_node
            .clone()
            .update_branch_at_index(
            Some(convert_h256_to_bytes(target_node_hash)),
            branch_index
        )?;
        new_stack.insert(0, updated_node);
        stack_to_delete.push(current_node);
        Ok((
            self,
            target_key,
            old_stack,
            new_stack,
            stack_to_delete,
        ))
    }
    /**
     *
     * Here we recurse over the new_stack and the to_delete_stack, saving the
     * former in to the database and removing the latter. Before putting the
     * final node in the database, its hash is used to update the trie root.
     *
     */
    fn update_trie_database(
        self,
        mut new_stack: NodeStack,
        mut stack_to_delete: NodeStack,
    ) -> Result<Self> {
        match stack_to_delete.len() > 0 {
            true => {
                let node = stack_to_delete.pop()?;
                trace!(
                    "Removing {} from database w/ hash: {}",
                    node.get_type(),
                    node.get_hash()?
                );
                self.remove_node_from_database(node)
                    .and_then(|new_self|
                        new_self.update_trie_database(
                            new_stack,
                            stack_to_delete,
                        )
                    )
            },
            false => match new_stack.len() {
                0 => Ok(self),
                1 => {
                    let node = new_stack.pop()?;
                    let next_root_hash = node.get_hash()?;
                    trace!(
                        "Putting new {} in database w/ hash: {}",
                        node.get_type(),
                        next_root_hash
                    );
                    self.put_node_in_database(node)
                        .and_then(|new_self| {
                            trace!("Updating root hash to {}", next_root_hash);
                            new_self.update_root_hash(next_root_hash)
                        })
                }
                _ => {
                    let node = new_stack.pop()?;
                    trace!(
                        "Putting new {} in database w/ hash: {}",
                        node.get_type(),
                        node.get_hash()?
                    );
                    self.put_node_in_database(node)
                        .and_then(|new_self|
                            new_self.update_trie_database(
                                new_stack,
                                stack_to_delete,
                            )
                        )
                }
            }
        }
    }

    fn find(
        self,
        target_key: Nibbles
    ) -> Result<(Self, Nibbles, NodeStack, Nibbles)> {
        get_node_from_database(&self.database, &self.root)
            .and_then(|maybe_node| match maybe_node {
                Some(node) => Trie::find_path(
                    self,
                    target_key.clone(),
                    vec![node],
                    target_key
                    ),
                None => Err(AppError::Custom(
                    "✘ Find Error: Could not find root node in db!".to_string()
                ))
            })
    }

    fn find_path(
        self,
        target_key: Nibbles,
        mut found_stack: NodeStack,
        remaining_key: Nibbles
    ) -> Result<(Self, Nibbles, NodeStack, Nibbles)> {
        match found_stack.pop() {
            None => {
                trace!("No node in top of stack");
                Ok((self, target_key, found_stack, remaining_key))
            },
            Some(current_node) => {
                match current_node.get_type() {
                    "leaf" => Self::continue_finding_from_leaf(
                        self,
                        target_key,
                        current_node,
                        found_stack,
                        remaining_key,
                    ),
                    "branch" => Self::continue_finding_from_branch(
                        self,
                        target_key,
                        current_node,
                        found_stack,
                        remaining_key
                    ),
                    "extension" => Self::continue_finding_from_extension(
                        self,
                        target_key,
                        current_node,
                        found_stack,
                        remaining_key,
                    ),
                    _ => Err(AppError::Custom(
                        "✘ Find Error: Node type not recognized!".to_string()
                    ))
                }
            }
        }
    }
    /**
     *
     * Finding onwards from a leaf node:
     *
     * Once at a leaf node we first check for any common prefix between our
     * target key and the leaf key. Once determined, we consider the two cases
     * of what remains of the target key:
     *
     * 1) No key remains.
     * 2) Some or all the key remains.
     *
     * In the first case, we have a full match and so return stack including
     * this leaf node along with an empty key.
     *
     * In case 2) we have no match but this is the closest node we got to. The
     * curent node is pushed back on the stack, which latter is returned along
     * with the key that remains to be found that was passed in.
     *
     */
    fn continue_finding_from_leaf(
        self,
        target_key: Nibbles,
        current_node: Node,
        mut found_stack: NodeStack,
        key: Nibbles
    ) -> Result<(Self, Nibbles, NodeStack, Nibbles)> {
        trace!("Leaf node found");
        get_common_prefix_nibbles(key.clone(), current_node.get_key())
            .and_then(|(_, remaining_key, _)| {
                found_stack.push(current_node);
                match remaining_key.len() {
                    0 => {
                        trace!("Wohoo! Leaf node matches fully!");
                        Ok((self, target_key, found_stack, EMPTY_NIBBLES))
                    },
                    _ => {
                        trace!("Leaf node has some | no match");
                        Ok((self, target_key, found_stack, key))
                    }
                }
            })
    }
    /**
     *
     * Finding onwards from an extension node:
     *
     * Once at an extension either we have three cases to consider:
     *
     * 1) No common prefix between target key and extension key.
     * 2) A common prefix that partially consumes the extension key.
     * 3) A common prefix that entirely consumes the extension key.
     *
     * In all three case we require the current node returned for further work.
     * In cases 1) & 2) we have reached the end of our search and so simply
     * return the stack of nodes and the key passed in.
     *
     * In case 3) we have fully consumed the extension node and so must get the
     * next node that the extension points to and add that to the stack. Then
     * pass that stack along with what remains of our target key for continued
     * searching.
     *
     */
    fn continue_finding_from_extension(
        self,
        target_key: Nibbles,
        current_node: Node,
        mut found_stack: NodeStack,
        key: Nibbles
    ) -> Result<(Self, Nibbles, NodeStack, Nibbles)> {
        trace!("Extension node found");
        get_common_prefix_nibbles(key.clone(), current_node.get_key())
            .and_then(|(common_prefix, remaining_key, remaining_node_key)| {
                let next_node_hash = &convert_bytes_to_h256(
                    &current_node.get_value()?
                )?;
                found_stack.push(current_node);
                match common_prefix.len() {
                    0 => {
                        trace!("Extension & key have no common prefix.");
                        Ok((self, target_key, found_stack, key))
                    },
                    _ => match remaining_node_key.len() > 0 {
                        true => {
                            trace!("Extension partial match.");
                            Ok((self, target_key, found_stack, key))
                        },
                        false => {
                            trace!("Extension full match, continuing...");
                            match get_node_from_database(
                                &self.database,
                                next_node_hash
                            )? {
                                Some(next_node) => {
                                    found_stack.push(next_node);
                                    Self::find_path(
                                        self,
                                        target_key,
                                        found_stack,
                                        remaining_key
                                    )
                                },
                                None => Err(AppError::Custom(
                                    "✘ Find Error: Extension child not in db!"
                                        .to_string()
                                ))
                            }
                        }
                    }
                }
            })
    }
    /**
     *
     * Finding onwards from a branch node:
     *
     * When arriving at a branch node, we take our target key and slice off the
     * first nibble. This is then used as the index for inspecting the branches
     * children, at which point there are two cases:
     *
     * 1) The child is empty.
     * 2) The child is not empty.
     *
     * In the first case, we have reached the end of our search. The branch node
     * is placed back in the stack which is then returned along with the target
     * key passed in.
     *
     * In the second case, we have two more cases:
     *
     * 1) The child is a hash.
     * 2) The child is an inline node.
     *
     * In the first case we search the database for the node pointed to by that
     * hash and then add it to the stack after first adding the branch node
     * we're currently looking at back to the stack. We then recurse back into
     * the `find_path` function with our updated stack and the target key.
     *
     * The second case is not yet currently handled. // TODO!
     *
     */
    fn continue_finding_from_branch(
        self,
        target_key: Nibbles,
        current_node: Node,
        mut found_stack: NodeStack,
        key: Nibbles
    ) -> Result<(Self, Nibbles, NodeStack, Nibbles)> {
        trace!("Branch node found");
        found_stack.push(current_node.clone());
        split_at_first_nibble(&key)
            .and_then(|(first_nibble, remaining_nibbles)| {
                match &current_node
                    .branch?
                    .branches[convert_nibble_to_usize(first_nibble)] {
                    None => {
                        trace!("No hash at next nibble index in branch");
                        Ok((self, target_key, found_stack, key))
                    },
                    Some(bytes) => {
                        match get_node_from_database(
                            &self.database,
                            &convert_bytes_to_h256(&bytes)?
                        )? {
                            Some(next_node) => {
                                trace!(
                                    "Next node retrieved from hash in {}",
                                    "branch, continuing..."
                                );
                                found_stack.push(next_node);
                                Self::find_path(
                                    self,
                                    target_key,
                                    found_stack,
                                    remaining_nibbles
                                )
                            },
                            None => Err(AppError::Custom(
                                "✘ Find Error: Branch child not in db!"
                                    .to_string()
                            )),
                        }
                    }
                }
        })
    }

    pub fn update_root_hash(mut self, new_hash: H256) -> Result<Self> {
        self.root = new_hash;
        Ok(self)
    }

    fn put_node_in_database(self, node: Node) -> Result<Self> {
        Ok(
            Trie {
                root: self.root,
                database: put_thing_in_database(
                    self.database,
                    node.get_hash()?,
                    node.get_rlp_encoding()?
                )?
            }
        )
    }

    fn remove_node_from_database(self, node: Node) -> Result<Self> {
        Ok(
            Trie {
                root: self.root,
                database: remove_thing_from_database(
                    self.database,
                    &node.get_hash()?,
                )?
            }
        )
    }

}

fn get_key_length_accounted_for_in_stack(node_stack: &NodeStack) -> usize {
    node_stack
        .iter()
        .map(|node| node.get_key_length())
        .sum()
}

pub fn put_in_trie_recursively(
    trie: Trie,
    key_value_tuples: Vec<(Nibbles, Bytes)>,
    i: usize
) -> Result<Trie> {
    match i == key_value_tuples.len()  {
        true => Ok(trie),
        false => {
            trace!("Putting item in trie recurisively...");
            trie
                .put(key_value_tuples[i].0.clone(), key_value_tuples[i].1.clone())
                .and_then(|new_trie|
                    put_in_trie_recursively(
                        new_trie,
                        key_value_tuples,
                        i + 1
                    )
                )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use super::*;
    use crate::types::Receipt;
    use crate::get_receipt::get_receipt_from_tx_hash;
    use crate::get_database::get_thing_from_database;
    use crate::rlp_codec::get_rlp_encoded_receipts_and_nibble_tuples;
    use crate::utils::{
        convert_hex_to_h256,
        convert_h256_to_prefixed_hex,
    };
    use crate::nibble_utils::{
        get_nibbles_from_bytes,
        convert_hex_string_to_nibbles,
    };
    use crate::test_utils::{
        RECEIPTS_ROOT,
        get_sample_receipts,
        get_sample_leaf_node,
        get_sample_branch_node,
        SAMPLE_RECEIPT_JSONS_PATH,
        SAMPLE_RECECIPT_TX_HASHES,
        get_sample_extension_node,
    };

    #[test]
    fn should_get_empty_trie() {
        let trie = Trie::get_new_trie()
            .unwrap();
        assert!(trie.database.is_empty());
        assert!(trie.root == HASHED_NULL_NODE);
    }

    #[test]
    fn should_put_thing_in_empty_trie() {
        let key = convert_hex_string_to_nibbles("c0ffe".to_string())
            .unwrap();
        let value = vec![0xde, 0xca, 0xff];
        let expected_node = Node::get_new_leaf_node(key.clone(), value.clone())
            .unwrap();
        let expected_db_key = expected_node
            .get_hash()
            .unwrap();
        let expected_thing_from_db = expected_node
            .get_rlp_encoding()
            .unwrap();
        let trie = Trie::get_new_trie()
            .unwrap();
        let result = trie.put(key, value)
            .unwrap();
        assert!(result.root == expected_node.get_hash().unwrap());
        let thing_from_db = get_thing_from_database(
            &result.database,
            &expected_db_key
        ).unwrap();
        assert!(thing_from_db == expected_thing_from_db)
    }

    #[test]
    fn should_update_root_hash() {
        let trie = Trie::get_new_trie()
            .unwrap();
        let old_hash = trie.root;
        let new_hash = convert_hex_to_h256(
            "a8780134f4add652b6e22e16a45b3436d3ecc293840fe8433f6fbcdc9ea8f16e".to_string()
        ).unwrap();
        assert!(old_hash != new_hash);
        let result = trie.update_root_hash(new_hash)
            .unwrap();
        assert!(result.root == new_hash);
        assert!(result.root != old_hash);
    }

    #[test]
    fn should_put_node_in_database_in_trie() {
        let node_key = convert_hex_string_to_nibbles("c0ffe".to_string())
            .unwrap();
        let node_value = vec![0xde, 0xca, 0xff];
        let trie = Trie::get_new_trie()
            .unwrap();
        let node = Node::get_new_leaf_node(node_key.clone(), node_value.clone())
            .unwrap();
        let expected_result = node.get_rlp_encoding()
            .unwrap();
        let node_hash = node
            .get_hash()
            .unwrap();
        let updated_trie = trie
            .put_node_in_database(node.clone())
            .unwrap();
        let result = get_thing_from_database(&updated_trie.database, &node_hash)
            .unwrap();
        assert!(result == expected_result);
    }

    #[test]
    fn should_remove_node_from_database() {
        let node_key = convert_hex_string_to_nibbles("c0ffe".to_string())
            .unwrap();
        let node_value = vec![0xde, 0xca, 0xff];
        let trie = Trie::get_new_trie()
            .unwrap();
        let node = Node::get_new_leaf_node(node_key.clone(), node_value.clone())
            .unwrap();
        let expected_result = node.get_rlp_encoding()
            .unwrap();
        let node_hash = node
            .get_hash()
            .unwrap();
        let updated_trie = trie
            .put_node_in_database(node.clone())
            .unwrap();
        assert!(updated_trie.database.contains_key(&node_hash));
        let resulting_trie = updated_trie.remove_node_from_database(node)
            .unwrap();
        assert!(!resulting_trie.database.contains_key(&node_hash));
    }

    #[test]
    fn should_sum_length_of_key_so_far_in_found_stack() {
        let mut found_stack: NodeStack = Vec::new();
        let leaf_node = get_sample_leaf_node();
        let branch_node = get_sample_branch_node();
        let extension_node = get_sample_extension_node();
        found_stack.push(leaf_node);
        found_stack.push(extension_node);
        found_stack.push(branch_node);
        let expected_result = 13;
        let result = get_key_length_accounted_for_in_stack(&found_stack);
        assert!(result == expected_result);
    }


    #[test]
    fn should_put_receipts_in_trie_correctly() {
        simple_logger::init().unwrap();
        let index = 0;
        let receipts = get_sample_receipts();
        let trie = Trie::get_new_trie().unwrap();
        let key_value_tuples = get_rlp_encoded_receipts_and_nibble_tuples(
            &receipts
        ).unwrap();
        let updated_trie = put_in_trie_recursively(
            trie,
            key_value_tuples,
            index
        ).unwrap();
        let root_hex = convert_h256_to_prefixed_hex(updated_trie.root)
            .unwrap();
        assert!(root_hex == RECEIPTS_ROOT);
    }
}
